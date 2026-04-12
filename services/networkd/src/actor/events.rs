//! Network event handling and interface failover for the network actor.

use netlib::interface::InterfaceName;
use netlib::link::LinkStateKind;

use super::state::{InterfaceSnapshot, InterfaceState, NetworkActor, NetworkStateKind};
use crate::monitor::NetworkEvent;

impl NetworkActor {
    pub(super) async fn handle_event(&mut self, event: NetworkEvent) {
        match event {
            NetworkEvent::LinkUp { name, index } => {
                self.on_link_up(name, index).await;
            }
            NetworkEvent::LinkDown { name, index } => {
                self.on_link_down(name, index).await;
            }
            NetworkEvent::LinkAdded { name, index, mac } => {
                self.on_link_added(name, index, mac).await;
            }
            NetworkEvent::LinkDeleted { name, index } => {
                self.on_link_deleted(name, index).await;
            }
        }
    }

    async fn on_link_up(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link up {} (index {})", name, index);

        let Some(iface) = self.get_interface_mut(name.as_str()) else {
            return;
        };
        iface.link = LinkStateKind::Up;
        self.transition_degraded_on_link_up(name.as_str());
        self.sync_and_publish();

        if self.is_primary_interface(&name) {
            self.handle_primary_recovery(&name);
        }
    }

    async fn on_link_down(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link down {} (index {})", name, index);

        let Some(iface) = self.get_interface_mut(name.as_str()) else {
            return;
        };
        iface.link = LinkStateKind::Down;

        if iface.state == InterfaceState::Configured
            && let Err(e) = iface.transition(InterfaceState::Degraded)
        {
            kmsg::warn!(
                "Interface {} state transition failed on link-down: {}",
                name,
                e
            );
        }

        self.sync_and_publish();

        if self.is_primary_interface(&name) {
            self.handle_primary_failure(&name);
        }
    }

    async fn on_link_added(&mut self, name: InterfaceName, index: u32, mac: [u8; 6]) {
        kmsg::info!(
            "Event: Link added {} (index {}, MAC {})",
            name,
            index,
            netlib::mac::format(&mac)
        );

        if self.has_interface(name.as_str()) {
            return;
        }

        let snapshot = InterfaceSnapshot {
            name: name.clone(),
            state: InterfaceState::Discovered,
            index,
            mac,
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
        };
        self.insert_interface(snapshot);

        if self.state.primary.is_none() {
            self.assign_as_primary(name);
        } else {
            self.add_to_backups(name);
        }

        self.sync_and_publish();
    }

    async fn on_link_deleted(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link deleted {} (index {})", name, index);

        if self.remove_interface(name.as_str()).is_none() {
            return;
        }

        if self.is_primary_interface(&name) {
            self.handle_primary_removed(&name);
        } else {
            self.remove_from_backups(name.as_str());
        }

        self.sync_and_publish();
    }

    fn is_primary_interface(&self, name: &InterfaceName) -> bool {
        self.state.primary.as_ref() == Some(name)
    }

    /// Transitions a `Degraded` interface to `Configured` (lease valid) or `Configuring` (no lease).
    fn transition_degraded_on_link_up(&mut self, iface: &str) {
        let Some(snap) = self.get_interface_mut(iface) else {
            return;
        };
        if snap.state != InterfaceState::Degraded {
            return;
        }
        let target = if snap.lease.is_some() {
            InterfaceState::Configured
        } else {
            InterfaceState::Configuring
        };
        if let Err(e) = snap.transition(target) {
            kmsg::warn!(
                "Interface {} state transition failed on link-up: {}",
                iface,
                e
            );
        }
    }

    fn handle_primary_recovery(&mut self, name: &InterfaceName) {
        kmsg::info!("Primary interface {} recovered", name);
        if let Err(e) = self.state.transition(NetworkStateKind::Operational) {
            kmsg::warn!("Unexpected state during primary recovery: {}", e);
        } else {
            self.publish_state();
        }
    }

    fn handle_primary_failure(&mut self, name: &InterfaceName) {
        kmsg::warn!("Primary interface {} failed", name);
        if let Err(e) = self.state.transition(NetworkStateKind::Degraded) {
            kmsg::warn!("Unexpected state during primary failure: {}", e);
        } else {
            self.publish_state();
        }

        self.try_failover_to_backup(name);
    }

    /// Promotes the first backup that is in `Configured` state to primary.
    fn try_failover_to_backup(&mut self, failed: &InterfaceName) {
        let Some(new_primary) = self
            .state
            .backups
            .iter()
            .find(|b| {
                self.iface_map
                    .get(b.as_str())
                    .is_some_and(|i| i.state == InterfaceState::Configured)
            })
            .cloned()
        else {
            kmsg::info!("No configured backup available for failover");
            return;
        };

        kmsg::info!("Failing over from {} to {}", failed, new_primary);
        self.state.backups.retain(|n| n != &new_primary);
        self.state.backups.push(failed.clone());
        self.state.primary = Some(new_primary.clone());
        if let Err(e) = self.state.transition(NetworkStateKind::Operational) {
            kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
        } else {
            self.publish_state();
        }
    }

    fn handle_primary_removed(&mut self, name: &InterfaceName) {
        kmsg::info!("Primary interface {} removed", name);

        if let Some(new_primary) = self.state.backups.first().cloned() {
            kmsg::info!("Promoting {} to primary", new_primary);
            self.state.primary = Some(new_primary.clone());
            self.state.backups.retain(|n| n != &new_primary);
        } else {
            self.clear_primary_and_degrade();
        }
    }

    /// Clears the primary interface and transitions the network state to degraded.
    fn clear_primary_and_degrade(&mut self) {
        kmsg::warn!("No backup interfaces available");
        self.state.primary = None;
        if let Err(e) = self.state.transition(NetworkStateKind::Degraded) {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }

    fn assign_as_primary(&mut self, name: InterfaceName) {
        kmsg::info!("Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    fn add_to_backups(&mut self, name: InterfaceName) {
        kmsg::info!("Adding {} to backup interfaces", name);
        self.state.backups.push(name);
    }

    fn remove_from_backups(&mut self, name: &str) {
        self.state.backups.retain(|n| n != name);
    }
}
