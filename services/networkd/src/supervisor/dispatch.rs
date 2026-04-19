//! Netlink event consumer: dispatches received `NetworkEvent`s to supervisor state handlers.

use netlib::interface::InterfaceName;
use netlib::monitor::NetworkEvent;
use netlib::ops::NetlinkOps;

use super::NetworkSupervisor;
use crate::interface::InterfaceCommand;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::state::InterfaceState;

impl<N: NetlinkOps> NetworkSupervisor<N> {
    /// Dispatches a netlink event to the appropriate handler.
    pub(super) async fn handle_event(&mut self, event: NetworkEvent) {
        match event {
            NetworkEvent::LinkUp { name, index } => self.on_link_up(name, index).await,
            NetworkEvent::LinkDown { name, index } => self.on_link_down(name, index).await,
            NetworkEvent::LinkAdded { name, index, mac } => {
                self.on_link_added(name, index, mac).await;
            }
            NetworkEvent::LinkDeleted { name, index } => self.on_link_deleted(name, index).await,
        }
    }

    async fn on_link_up(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link up {} (index {})", name, index);
        self.send_to_interface(&name, InterfaceCommand::LinkUp)
            .await;
        if self.is_primary_interface(&name) {
            self.handle_primary_recovery(&name);
        } else if self.state.backups.contains(&name) {
            self.handle_backup_recovery(&name);
        }
    }

    async fn on_link_down(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link down {} (index {})", name, index);
        self.send_to_interface(&name, InterfaceCommand::LinkDown)
            .await;
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

        if self.interfaces.contains_key(&name) {
            return;
        }

        let snapshot = InterfaceSnapshot {
            name: name.clone(),
            state: InterfaceState::Discovered,
            index,
            mac,
            link: netlib::link::LinkStateKind::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
            l3_owner: name.clone(),
        };
        self.spawn_interface_actor(snapshot);

        if self.state.primary.is_none() {
            self.assign_as_primary(name);
        } else {
            self.add_to_backups(name);
        }

        self.sync_and_publish();
    }

    async fn on_link_deleted(&mut self, name: InterfaceName, index: u32) {
        kmsg::info!("Event: Link deleted {} (index {})", name, index);

        let Some(actor_handle) = self.interfaces.remove(&name) else {
            return;
        };
        let _ = actor_handle.cmd_tx.send(InterfaceCommand::Shutdown).await;

        if self.is_primary_interface(&name) {
            self.handle_primary_removed(&name);
        } else {
            self.remove_from_backups(name.as_str());
        }

        self.sync_and_publish();
    }

    pub(super) fn assign_as_primary(&mut self, name: InterfaceName) {
        kmsg::info!("Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    pub(super) fn add_to_backups(&mut self, name: InterfaceName) {
        kmsg::info!("Adding {} to backup interfaces", name);
        let pos = self
            .state
            .backups
            .partition_point(|existing| existing < &name);
        self.state.backups.insert(pos, name);
    }

    pub(super) fn remove_from_backups(&mut self, name: &str) {
        self.state.backups.retain(|n| n != name);
    }
}
