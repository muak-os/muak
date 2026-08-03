//! Primary/backup promotion and failover policy for the network supervisor.

use netlib::interface::Name;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use crate::interface::state::Lifecycle;
use crate::supervisor::state::NetworkState;

impl<N: Ops> NetworkSupervisor<N> {
    pub(super) fn is_primary_interface(&self, name: &Name) -> bool {
        self.state.primary.as_ref() == Some(name)
    }

    pub(super) fn is_interface_configured(&self, name: &Name) -> bool {
        self.interfaces
            .get(name)
            .is_some_and(|handle| handle.state_rx.borrow().state == Lifecycle::Configured)
    }

    pub(super) fn handle_primary_recovery(&mut self, name: &Name) {
        kmsg::info!("Primary interface {} recovered", name);
        if let Err(e) = self.state.transition(NetworkState::Operational) {
            kmsg::warn!("Unexpected state during primary recovery: {}", e);
        } else {
            self.publish_state();
        }
    }

    /// Restores a recovered backup as primary if it was previously the primary.
    pub(super) fn handle_backup_recovery(&mut self, recovered: &Name) {
        let Some(current_primary) = self.state.primary.clone() else {
            return;
        };

        kmsg::info!(
            "Recovered interface {} restoring as primary (demoting {})",
            recovered,
            current_primary
        );

        self.state.backups.retain(|n| n != recovered);
        self.state.backups.push(current_primary);
        self.state.primary = Some(recovered.clone());

        if let Err(e) = self.state.transition(NetworkState::Operational) {
            kmsg::warn!("Unexpected state restoring primary {}: {}", recovered, e);
        } else {
            self.publish_state();
        }
    }

    pub(super) fn handle_primary_failure(&mut self, name: &Name) {
        kmsg::warn!("Primary interface {} failed", name);
        if let Err(e) = self.state.transition(NetworkState::Degraded) {
            kmsg::warn!("Unexpected state during primary failure: {}", e);
        } else {
            self.publish_state();
        }

        self.try_failover_to_backup(name);
    }

    pub(super) fn handle_primary_removed(&mut self, name: &Name) {
        kmsg::info!("Primary interface {} removed", name);

        if let Some(new_primary) = self.state.backups.first().cloned() {
            kmsg::info!("Promoting {} to primary", new_primary);
            self.state.primary = Some(new_primary.clone());
            self.state.backups.retain(|n| n != &new_primary);
        } else {
            self.clear_primary_and_degrade();
        }
    }

    /// Promotes the first backup that is in `Configured` state to primary.
    fn try_failover_to_backup(&mut self, failed: &Name) {
        let Some(new_primary) = self
            .state
            .backups
            .iter()
            .find(|backup| self.is_interface_configured(backup))
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

    fn clear_primary_and_degrade(&mut self) {
        kmsg::warn!("No backup interfaces available");
        self.state.primary = None;
        if let Err(e) = self.state.transition(NetworkState::Degraded) {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }
}
