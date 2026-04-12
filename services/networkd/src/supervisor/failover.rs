//! Primary/backup promotion and failover policy for the network supervisor.

use netlib::interface::InterfaceName;

use super::NetworkSupervisor;
use crate::interface::state::InterfaceState;
use crate::supervisor::state::NetworkState;

impl NetworkSupervisor {
    pub(super) fn is_primary_interface(&self, name: &InterfaceName) -> bool {
        self.state.primary.as_ref() == Some(name)
    }

    pub(super) fn is_interface_configured(&self, name: &InterfaceName) -> bool {
        self.interfaces
            .get(name)
            .is_some_and(|h| h.state_rx.borrow().state == InterfaceState::Configured)
    }

    pub(super) fn handle_primary_recovery(&mut self, name: &InterfaceName) {
        kmsg::info!("Primary interface {} recovered", name);
        if let Err(e) = self.state.transition(NetworkState::Operational) {
            kmsg::warn!("Unexpected state during primary recovery: {}", e);
        } else {
            self.publish_state();
        }
    }

    pub(super) fn handle_primary_failure(&mut self, name: &InterfaceName) {
        kmsg::warn!("Primary interface {} failed", name);
        if let Err(e) = self.state.transition(NetworkState::Degraded) {
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
            .find(|b| self.is_interface_configured(b))
            .cloned()
        else {
            kmsg::info!("No configured backup available for failover");
            return;
        };

        kmsg::info!("Failing over from {} to {}", failed, new_primary);
        self.state.backups.retain(|n| n != &new_primary);
        self.state.backups.push(failed.clone());
        self.state.primary = Some(new_primary.clone());
        if let Err(e) = self.state.transition(NetworkState::Operational) {
            kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
        } else {
            self.publish_state();
        }
    }

    pub(super) fn handle_primary_removed(&mut self, name: &InterfaceName) {
        kmsg::info!("Primary interface {} removed", name);

        if let Some(new_primary) = self.state.backups.first().cloned() {
            kmsg::info!("Promoting {} to primary", new_primary);
            self.state.primary = Some(new_primary.clone());
            self.state.backups.retain(|n| n != &new_primary);
        } else {
            self.clear_primary_and_degrade();
        }
    }

    fn clear_primary_and_degrade(&mut self) {
        kmsg::warn!("No backup interfaces available");
        self.state.primary = None;
        if let Err(e) = self.state.transition(NetworkState::Degraded) {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }
}
