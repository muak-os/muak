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

            // Process recovery if:
            // 1. We're not on the primary (active != primary) AND the primary interface is coming back up
            if self.is_primary_interface(&name) && !self.state.is_on_primary() {
                log!("network", "Primary interface {} is back up while running on secondary - checking recovery", name);
                self.handle_primary_recovery(&name);
            }
        }
    }

    async fn on_link_down(&mut self, name: String, index: u32) {
        log!("network", "Event: Link down {} (index {})", name, index);

        if let Some(iface) = self.get_interface_mut(&name) {
            iface.link = LinkStateKind::Down;
            self.sync_and_publish();

            // Only process failover if network is in Ready state and this is the active interface
            if self.state.state == NetworkStateKind::Ready && self.is_active_interface(&name) {
                // During the first few seconds after becoming Ready, check if interface is enslaved
                // to filter out queued events from bridge setup. After that grace period, always trigger failover.
                let grace_period_active = self.ready_at
                    .map(|t| t.elapsed() < std::time::Duration::from_secs(5))
                    .unwrap_or(false);

                if grace_period_active {
                    if let Ok(is_enslaved) = self.is_interface_enslaved(&name).await {
                        if is_enslaved {
                            log!("network", "Ignoring link down for enslaved interface {} (grace period)", name);
                            return;
                        }
                    }
                }

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
                self.add_to_secondaries(name);
            }

            self.sync_and_publish();
        }
    }

    async fn on_link_deleted(&mut self, name: String, index: u32) {
        log!("network", "Event: Link deleted {} (index {})", name, index);

        if self.remove_interface(&name).is_some() {
            if self.is_active_interface(&name) {
                self.handle_primary_removed(&name);
            } else {
                self.remove_from_secondaries(&name);
            }

            self.sync_and_publish();
        }
    }

    fn is_active_interface(&self, name: &str) -> bool {
        self.state.active.as_ref() == Some(&name.to_string())
    }

    fn is_primary_interface(&self, name: &str) -> bool {
        self.state.primary.as_ref() == Some(&name.to_string())
    }

    fn handle_primary_recovery(&mut self, name: &str) {
        // Only trigger recovery if we're not currently on the primary
        if !self.state.is_on_primary() {
            log!("network", "Primary interface {} recovered - initiating bridge migration", name);
            
            // Find current active interface to migrate from
            if let Some(current_active) = self.state.active.clone() {
                log!(
                    "network",
                    "Migrating bridge from active {} back to primary {}",
                    current_active,
                    name
                );
                self.trigger_recovery_migration(current_active, name.to_string());
            } else {
                // No secondary was promoted, just restore operational state
                log!("network", "Primary {} recovered, restoring operational state", name);
                self.state.state = NetworkStateKind::Operational;
                self.publish_state();
            }
        }
    }

    fn handle_primary_failure(&mut self, name: &str) {
        log!("network", "Active interface {} failed (was primary)", name);
        self.state.state = NetworkStateKind::Degraded;
        
        // Move failed primary to secondaries so it can be recovered later
        if self.state.primary.as_deref() == Some(name) && !self.state.secondaries.contains(&name.to_string()) {
            self.state.secondaries.push(name.to_string());
            log!("network", "Moved failed primary {} to secondaries for recovery tracking", name);
        }
        
        self.publish_state();

        if let Some(secondary) = self.state.secondaries.iter()
            .find(|s| *s != name)
            .cloned() {
            log!(
                "network",
                "Initiating automatic failover from {} to secondary {}",
                name,
                secondary
            );
            self.trigger_failover(secondary);
        } else {
            log!("network", "No secondary interfaces available for failover");
        }
    }

    fn handle_primary_removed(&mut self, name: &str) {
        log!("network", "Primary interface {} removed", name);

        if let Some(new_active) = self.state.secondaries.first().cloned() {
            log!("network", "Selecting {} as new active", new_active);
            self.state.active = Some(new_active.clone());
            self.state.primary = Some(new_active.clone());
            self.state.secondaries.retain(|n| n != &new_active);
        } else {
            log!("network", "No secondary interfaces available");
            self.state.active = None;
            self.state.primary = None;
            self.state.state = NetworkStateKind::Degraded;
        }
    }

    fn assign_as_primary(&mut self, name: String) {
        log!("network", "Assigning {} as primary interface", name);
        self.state.primary = Some(name);
    }

    fn add_to_secondaries(&mut self, name: String) {
        log!("network", "Adding {} to secondary interfaces", name);
        self.state.secondaries.push(name);
    }

    fn remove_from_secondaries(&mut self, name: &str) {
        self.state.secondaries.retain(|n| n != name);
    }

    fn trigger_failover(&mut self, secondary: String) {
        use super::commands::NetworkCommand;

        let cmd_tx = self.get_command_sender();
        tokio::spawn(async move {
            let _ = cmd_tx
                .send(NetworkCommand::PromoteSecondary { secondary })
                .await;
        });
    }

    fn trigger_recovery_migration(&mut self, from_secondary: String, to_primary: String) {
        use super::commands::NetworkCommand;

        let cmd_tx = self.get_command_sender();
        tokio::spawn(async move {
            let _ = cmd_tx
                .send(NetworkCommand::RecoverPrimary {
                    from_secondary,
                    to_primary,
                })
                .await;
        });
    }
}
