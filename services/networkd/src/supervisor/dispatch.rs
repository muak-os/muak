//! Netlink event consumer: dispatches received `NetworkEvent`s to supervisor state handlers.

use netlib::interface::Name;
use netlib::link::State;
use netlib::mac::format as format_mac;
use netlib::monitor::Event;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;
use crate::interface::state::Lifecycle;

impl<N: Ops> NetworkSupervisor<N> {
    /// Dispatches a netlink event to the appropriate handler.
    pub(super) async fn handle_event(&mut self, event: Event) {
        match event {
            Event::Up { name, index } => self.on_link_up(name, index).await,
            Event::Down { name, index } => self.on_link_down(name, index).await,
            Event::Added { name, index, mac } => {
                self.on_link_added(name, index, mac);
            }
            Event::Deleted { name, index } => self.on_link_deleted(name, index).await,
        }
    }

    pub(super) fn assign_as_primary(&mut self, name: Name) {
        kmsg::info!("Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    pub(super) fn add_to_backups(&mut self, name: Name) {
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

    async fn on_link_up(&mut self, name: Name, index: u32) {
        kmsg::info!("Event: Link up {} (index {})", name, index);
        self.send_to_interface(&name, Command::LinkUp).await;
        if self.is_primary_interface(&name) {
            self.handle_primary_recovery(&name);
            return;
        }
        if self.state.backups.contains(&name) {
            self.handle_backup_recovery(&name);
        }
    }

    async fn on_link_down(&mut self, name: Name, index: u32) {
        kmsg::info!("Event: Link down {} (index {})", name, index);
        self.send_to_interface(&name, Command::LinkDown).await;
        if self.is_primary_interface(&name) {
            self.handle_primary_failure(&name);
        }
    }

    fn on_link_added(&mut self, name: Name, index: u32, mac: [u8; 6]) {
        kmsg::info!(
            "Event: Link added {} (index {}, MAC {})",
            name,
            index,
            format_mac(&mac)
        );

        if self.interfaces.contains_key(&name) {
            return;
        }

        let snapshot = Snapshot {
            name: name.clone(),
            state: Lifecycle::Discovered,
            index,
            mac,
            link: State::Up,
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

    async fn on_link_deleted(&mut self, name: Name, index: u32) {
        kmsg::info!("Event: Link deleted {} (index {})", name, index);

        let Some(actor_handle) = self.interfaces.remove(&name) else {
            return;
        };
        drop(actor_handle.cmd_tx.send(Command::Shutdown).await);

        if self.is_primary_interface(&name) {
            self.handle_primary_removed(&name);
        } else {
            self.remove_from_backups(name.as_str());
        }

        self.sync_and_publish();
    }
}
