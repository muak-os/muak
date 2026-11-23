//! L2 enslavement ownership for ports provisioned by the supervisor.

use alloc::vec::Vec;

use anyhow::Result;
use netdev::Ops;
use netlink::link::Failure;
use netlink::link::ifname::Name;

use super::super::NetworkSupervisor;

/// Records that this supervisor enslaved `port` to `master`.
pub(super) fn record<N: Ops>(supervisor: &mut NetworkSupervisor<N>, port: &Name, master: &Name) {
    supervisor.enslaved.insert(port.clone(), master.clone());
}

/// Detaches the ports this supervisor enslaved to `master` that are not in `desired`.
pub(super) async fn release_stale<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    master: &Name,
    master_index: u32,
    desired: &[Name],
) -> Result<()> {
    let stale: Vec<Name> = supervisor
        .enslaved
        .iter()
        .filter(|&(port, owner)| owner == master && !desired.contains(port))
        .map(|(port, _)| port.clone())
        .collect();

    for port in stale {
        release(supervisor, &port, master_index).await?;
    }

    Ok(())
}

async fn release<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    port: &Name,
    master_index: u32,
) -> Result<()> {
    let index = match supervisor.ops.index(port.as_str()).await {
        Ok(index) => index,
        Err(Failure::NotFound(_)) => {
            supervisor.enslaved.remove(port);
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    if supervisor.ops.master(index).await? == Some(master_index) {
        supervisor.ops.detach(index).await?;
    }
    supervisor.enslaved.remove(port);

    Ok(())
}
