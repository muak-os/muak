//! Carrier-driven link reconciliation for the interface that owns L3.

use netdev::Ops;

use super::{NetworkSupervisor, failover};
use crate::interface::commands::Command;
use crate::interface::lifecycle::Lifecycle;

/// Aligns the L3 owner's published link with the kernel after a topology change.
pub(super) async fn refresh_owner<N: Ops>(supervisor: &NetworkSupervisor<N>) {
    let Some(owner) = failover::l3_owner(&supervisor.state, &supervisor.interfaces).cloned() else {
        return;
    };
    let Some(handle) = supervisor.interfaces.get(&owner) else {
        return;
    };
    let (index, published, recovering) = {
        let snapshot = handle.state_rx.borrow();
        let recovering = matches!(snapshot.state, Lifecycle::Failed | Lifecycle::Degraded);
        (snapshot.index, snapshot.link, recovering)
    };

    let devices = match supervisor.ops.devices().await {
        Ok(devices) => devices,
        Err(e) => {
            kmsg::warn!("Failed to query devices while refreshing {owner}: {e}");
            return;
        }
    };
    let Some(device) = devices.iter().find(|device| device.index == index) else {
        kmsg::warn!("Link {owner} (index {index}) not found while refreshing carrier");
        return;
    };

    if published.has_carrier() == device.carrier {
        if !(device.carrier && recovering) {
            return;
        }
    }
    let command = if device.carrier {
        Command::LinkUp
    } else {
        Command::LinkDown
    };
    supervisor.send_to_interface(&owner, command).await;
}
