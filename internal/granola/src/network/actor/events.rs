use crate::log;
use crate::network::model::{InterfaceSnapshot, LinkStateKind, NetworkStateKind};
use crate::network::monitor::NetworkEvent;

use super::state::NetworkActor;

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
        log!("network", "Event: Link up {} (index {})", name, index);

        if let Some(iface) = self.get_interface_mut(&name) {
            iface.link = LinkStateKind::Up;
            self.sync_and_publish();

            if self.is_primary_interface(&name) {
                self.handle_primary_recovery(&name);
            }
        }
    }

    async fn on_link_down(&mut self, name: String, index: u32) {
        log!("network", "Event: Link down {} (index {})", name, index);

        if let Some(iface) = self.get_interface_mut(&name) {
            iface.link = LinkStateKind::Down;
            self.sync_and_publish();

            if self.is_primary_interface(&name) {
                self.handle_primary_failure(&name);
            }
        }
    }

    async fn on_link_added(&mut self, name: String, index: u32, mac: [u8; 6]) {
        log!(
            "network",
            "Event: Link added {} (index {}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
            name,
            index,
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );

        if !self.has_interface(&name) {
            let snapshot = InterfaceSnapshot {
                name: name.clone(),
                index,
                mac,
                link: LinkStateKind::Up,
                ip: None,
                lease: None,
            };
            self.insert_interface(snapshot);

            if self.state.primary.is_none() {
                self.assign_as_primary(name);
            } else {
                self.add_to_backups(name);
            }

            self.sync_and_publish();
        }
    }

    async fn on_link_deleted(&mut self, name: String, index: u32) {
        log!("network", "Event: Link deleted {} (index {})", name, index);

        if self.remove_interface(&name).is_some() {
            if self.is_primary_interface(&name) {
                self.handle_primary_removed(&name);
            } else {
                self.remove_from_backups(&name);
            }

            self.sync_and_publish();
        }
    }

    fn is_primary_interface(&self, name: &str) -> bool {
        self.state.primary.as_ref() == Some(&name.to_string())
    }

    fn handle_primary_recovery(&mut self, name: &str) {
        if self.state.state == NetworkStateKind::Degraded {
            log!("network", "Primary interface {} recovered", name);
            self.state.state = NetworkStateKind::Operational;
            self.publish_state();
        }
    }

    fn handle_primary_failure(&mut self, name: &str) {
        log!("network", "Primary interface {} failed", name);
        self.state.state = NetworkStateKind::Degraded;
        self.publish_state();

        if let Some(new_primary) = self.state.backups.first().cloned() {
            log!(
                "network",
                "Initiating automatic failover from {} to {}",
                name,
                new_primary
            );
            self.trigger_failover(new_primary);
        } else {
            log!("network", "No backup interfaces available for failover");
        }
    }

    fn handle_primary_removed(&mut self, name: &str) {
        log!("network", "Primary interface {} removed", name);

        if let Some(new_primary) = self.state.backups.first().cloned() {
            log!("network", "Promoting {} to primary", new_primary);
            self.state.primary = Some(new_primary.clone());
            self.state.backups.retain(|n| n != &new_primary);
        } else {
            log!("network", "No backup interfaces available");
            self.state.primary = None;
            self.state.state = NetworkStateKind::Degraded;
        }
    }

    fn assign_as_primary(&mut self, name: String) {
        log!("network", "Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    fn add_to_backups(&mut self, name: String) {
        log!("network", "Adding {} to backup interfaces", name);
        self.state.backups.push(name);
    }

    fn remove_from_backups(&mut self, name: &str) {
        self.state.backups.retain(|n| n != name);
    }

    fn trigger_failover(&mut self, new_primary: String) {
        use super::commands::NetworkCommand;
        use tokio::sync::mpsc;

        let cmd_tx = self.get_command_sender();
        tokio::spawn(async move {
            let _ = cmd_tx
                .send(NetworkCommand::PromotePrimary { new_primary })
                .await;
        });
    }
}
