//! Primary/backup promotion and failover policy for the network supervisor.

use netlib::interface::Name;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use crate::interface::state::Lifecycle;
use crate::supervisor::state::NetworkState;

pub(super) fn is_primary_interface<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    supervisor.state.primary.as_ref() == Some(name)
}

pub(super) fn is_interface_configured<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    name: &Name,
) -> bool {
    supervisor
        .interfaces
        .get(name)
        .is_some_and(|handle| handle.state_rx.borrow().state == Lifecycle::Configured)
}

pub(super) fn handle_primary_recovery<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::info!("Primary interface {} recovered", name);
    if let Err(e) = supervisor.state.transition(NetworkState::Operational) {
        kmsg::warn!("Unexpected state during primary recovery: {}", e);
    } else {
        supervisor.publish_state();
    }
}

/// Restores a recovered backup as primary if it was previously the primary.
pub(super) fn handle_backup_recovery<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    recovered: &Name,
) {
    let Some(current_primary) = supervisor.state.primary.clone() else {
        return;
    };

    kmsg::info!(
        "Recovered interface {} restoring as primary (demoting {})",
        recovered,
        current_primary
    );

    supervisor.state.backups.retain(|n| n != recovered);
    supervisor.state.backups.push(current_primary);
    supervisor.state.primary = Some(recovered.clone());

    if let Err(e) = supervisor.state.transition(NetworkState::Operational) {
        kmsg::warn!("Unexpected state restoring primary {}: {}", recovered, e);
    } else {
        supervisor.publish_state();
    }
}

pub(super) fn handle_primary_failure<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::warn!("Primary interface {} failed", name);
    if let Err(e) = supervisor.state.transition(NetworkState::Degraded) {
        kmsg::warn!("Unexpected state during primary failure: {}", e);
    } else {
        supervisor.publish_state();
    }

    try_failover_to_backup(supervisor, name);
}

pub(super) fn handle_primary_removed<N: Ops>(supervisor: &mut NetworkSupervisor<N>, name: &Name) {
    kmsg::info!("Primary interface {} removed", name);

    if let Some(new_primary) = supervisor.state.backups.first().cloned() {
        kmsg::info!("Promoting {} to primary", new_primary);
        supervisor.state.primary = Some(new_primary.clone());
        supervisor.state.backups.retain(|n| n != &new_primary);
    } else {
        kmsg::warn!("No backup interfaces available");
        supervisor.state.primary = None;
        if let Err(e) = supervisor.state.transition(NetworkState::Degraded) {
            kmsg::warn!("Unexpected state during primary removal: {}", e);
        }
    }
}

/// Promotes the first backup that is in `Configured` state to primary.
fn try_failover_to_backup<N: Ops>(supervisor: &mut NetworkSupervisor<N>, failed: &Name) {
    let Some(new_primary) = supervisor
        .state
        .backups
        .iter()
        .find(|backup| is_interface_configured(supervisor, backup))
        .cloned()
    else {
        kmsg::info!("No configured backup available for failover");
        return;
    };

    kmsg::info!("Failing over from {} to {}", failed, new_primary);
    supervisor.state.backups.retain(|n| n != &new_primary);
    supervisor.state.backups.push(failed.clone());
    supervisor.state.primary = Some(new_primary.clone());
    if let Err(e) = supervisor.state.transition(NetworkState::Operational) {
        kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
    } else {
        supervisor.publish_state();
    }
}
