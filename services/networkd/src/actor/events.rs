use super::state::NetworkActor;
use crate::model::{InterfaceSnapshot, LinkStateKind, NetworkStateKind};
use crate::monitor::NetworkEvent;
use crate::netutil::format_mac_address;

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

    async fn on_link_up(&mut self, name: String, index: u32) {
        kmsg::info!("Event: Link up {} (index {})", name, index);

        let Some(iface) = self.get_interface_mut(&name) else {
            return;
        };
        iface.link = LinkStateKind::Up;
        self.sync_and_publish();

        if self.is_primary_interface(&name) {
            self.handle_primary_recovery(&name);
        }
    }

    async fn on_link_down(&mut self, name: String, index: u32) {
        kmsg::info!("Event: Link down {} (index {})", name, index);

        let Some(iface) = self.get_interface_mut(&name) else {
            return;
        };
        iface.link = LinkStateKind::Down;
        self.sync_and_publish();

        if self.is_primary_interface(&name) {
            self.handle_primary_failure(&name);
        }
    }

    async fn on_link_added(&mut self, name: String, index: u32, mac: [u8; 6]) {
        kmsg::info!(
            "Event: Link added {} (index {}, MAC {})",
            name,
            index,
            format_mac_address(&mac)
        );

        if self.has_interface(&name) {
            return;
        }

        let snapshot = InterfaceSnapshot {
            name: name.clone(),
            index,
            mac,
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
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

    async fn on_link_deleted(&mut self, name: String, index: u32) {
        kmsg::info!("Event: Link deleted {} (index {})", name, index);

        if self.remove_interface(&name).is_none() {
            return;
        }

        if self.is_primary_interface(&name) {
            self.handle_primary_removed(&name);
        } else {
            self.remove_from_backups(&name);
        }

        self.sync_and_publish();
    }

    fn is_primary_interface(&self, name: &str) -> bool {
        self.state.primary.as_ref() == Some(&name.to_string())
    }

    fn handle_primary_recovery(&mut self, name: &str) {
        if self.state.state == NetworkStateKind::Degraded {
            kmsg::info!("Primary interface {} recovered", name);
            self.state.state = NetworkStateKind::Operational;
            self.publish_state();
        }
    }

    fn handle_primary_failure(&mut self, name: &str) {
        kmsg::warn!("Primary interface {} failed", name);
        self.state.state = NetworkStateKind::Degraded;
        self.publish_state();

        if !self.state.backups.is_empty() {
            // TODO: Here is where we could trigger a failover to a backup interface.
            kmsg::info!("Backup interfaces available: {:?}", self.state.backups);
        }
    }

    fn handle_primary_removed(&mut self, name: &str) {
        kmsg::info!("Primary interface {} removed", name);

        if let Some(new_primary) = self.state.backups.first().cloned() {
            kmsg::info!("Promoting {} to primary", new_primary);
            self.state.primary = Some(new_primary.clone());
            self.state.backups.retain(|n| n != &new_primary);
        } else {
            kmsg::warn!("No backup interfaces available");
            self.state.primary = None;
            self.state.state = NetworkStateKind::Degraded;
        }
    }

    fn assign_as_primary(&mut self, name: String) {
        kmsg::info!("Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    fn add_to_backups(&mut self, name: String) {
        kmsg::info!("Adding {} to backup interfaces", name);
        self.state.backups.push(name);
    }

    fn remove_from_backups(&mut self, name: &str) {
        self.state.backups.retain(|n| n != name);
    }
}
