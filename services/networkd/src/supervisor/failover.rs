//! Primary/backup promotion and failover policy for the network supervisor.

use netlib::interface::Name;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use crate::interface::commands::{ApplyMode, Command};
use crate::interface::state::Lifecycle;
use crate::supervisor::state::NetworkState;

pub(super) fn is_primary_interface<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> bool {
    supervisor.state.primary.as_ref() == Some(name)
}

/// Returns true when the elected primary's interface is in `Failed` state.
pub(super) fn is_primary_failed<N: Ops>(supervisor: &NetworkSupervisor<N>) -> bool {
    supervisor
        .state
        .primary
        .as_ref()
        .and_then(|primary| supervisor.interfaces.get(primary))
        .is_some_and(|handle| handle.state_rx.borrow().state == Lifecycle::Failed)
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

/// Restores a recovered backup as primary, unless the current primary is healthy.
pub(super) fn handle_backup_recovery<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    recovered: &Name,
) {
    let Some(current_primary) = supervisor.state.primary.clone() else {
        return;
    };

    if is_interface_configured(supervisor, &current_primary) {
        kmsg::info!(
            "Backup {} recovered; keeping configured primary {}",
            recovered,
            current_primary
        );
        return;
    }
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

pub(super) async fn handle_primary_failure<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    name: &Name,
) {
    kmsg::warn!("Primary interface {} failed", name);
    if let Err(e) = supervisor.state.transition(NetworkState::Degraded) {
        kmsg::warn!("Unexpected state during primary failure: {}", e);
    } else {
        supervisor.publish_state();
    }

    try_failover_to_backup(supervisor, name).await;
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

/// Promotes the highest-priority backup to primary and provisions it with DHCP.
async fn try_failover_to_backup<N: Ops>(supervisor: &mut NetworkSupervisor<N>, failed: &Name) {
    let configured_backup = supervisor
        .state
        .backups
        .iter()
        .find(|backup| is_interface_configured(supervisor, backup))
        .cloned();
    let Some(new_primary) = configured_backup.or_else(|| supervisor.state.backups.first().cloned())
    else {
        kmsg::info!("No backup available for failover");
        return;
    };

    kmsg::info!("Failing over from {} to {}", failed, new_primary);
    supervisor.state.backups.retain(|n| n != &new_primary);
    supervisor.state.backups.push(failed.clone());
    supervisor.state.primary = Some(new_primary.clone());

    if !is_interface_configured(supervisor, &new_primary) {
        supervisor
            .send_to_interface(
                &new_primary,
                Command::ConfigureDhcp {
                    mode: ApplyMode::Provision,
                },
            )
            .await;
    }

    if let Err(e) = supervisor.state.transition(NetworkState::Operational) {
        kmsg::warn!("Unexpected state after failover to {}: {}", new_primary, e);
    } else {
        supervisor.publish_state();
    }
}
