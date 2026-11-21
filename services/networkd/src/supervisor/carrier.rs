//! Carrier-driven link reconciliation for the interface that owns L3.

use netdev::Ops;
use netstack::interface::Command;

use super::{NetworkSupervisor, owner, view};

/// Aligns the L3 owner's published link with the kernel after a topology change.
pub(super) async fn refresh_owner<N: Ops>(supervisor: &NetworkSupervisor<N>) {
    let Some(owner) = owner::l3_owner(
        &supervisor.state,
        &supervisor.interfaces,
        &supervisor.owners,
    )
    .cloned() else {
        return;
    };
    let Some(handle) = supervisor.interfaces.get(&owner) else {
        return;
    };
    let (published, recovering) = {
        let snapshot = handle.snapshot_rx.borrow();
        (snapshot.link, view::is_degraded(&snapshot))
    };

    let index = match supervisor.ops.index(owner.as_str()).await {
        Ok(index) => index,
        Err(e) => {
            kmsg::warn!("Failed to query index of {owner}: {e}");
            return;
        }
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

    if published.has_carrier() == device.carrier && !(device.carrier && recovering) {
        return;
    }
    let command = if device.carrier {
        Command::LinkUp
    } else {
        Command::LinkDown
    };
    supervisor.send_to_interface(&owner, command).await;
}
