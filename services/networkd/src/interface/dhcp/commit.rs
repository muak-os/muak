//! Lease application, persistence and DHCP state advancement.

use core::net::Ipv4Addr;

use anyhow::Result;
use dhcp_client::lease::Lease;
use netdev::Ops;
use netlink::address::IpConfig;

use super::Actor;
use crate::interface::lifecycle::DhcpState;
use crate::statemachine::StateMachine as _;

/// Applies the network-level changes from a lease.
pub(super) async fn apply<N: Ops>(actor: &mut Actor<N>, index: u32, lease: &Lease) -> Result<()> {
    actor
        .ops
        .ensure_ipv4(index, lease.assigned_ip, lease.prefix_len)
        .await?;

    if let Some(gw) = lease.gateway {
        actor.ops.ensure_default_route(gw).await?;
    } else {
        kmsg::info!(
            "No gateway in DHCP lease on {}, skipping default route",
            actor.snapshot.name
        );
    }

    Ok(())
}

/// Persists the lease into the snapshot and publishes it.
pub(super) fn store<N: Ops>(actor: &mut Actor<N>, lease: &Lease) {
    actor.snapshot.ip = Some(IpConfig {
        address: lease.assigned_ip,
        prefix_len: lease.prefix_len,
        gateway: lease.gateway,
        dns: lease.dns_servers.clone(),
    });
    actor.snapshot.lease = Some(lease.clone());
    actor.publish_snapshot();
}

pub(super) fn transition<N: Ops>(actor: &mut Actor<N>, next: DhcpState) {
    let Some(current) = actor.snapshot.dhcp_state.as_mut() else {
        actor.snapshot.dhcp_state = Some(next);
        return;
    };
    if let Err(e) = current.transition(next) {
        kmsg::warn!(
            "DHCP state transition rejected on {}: {}",
            actor.snapshot.name,
            e
        );
    }
}

/// Applies kernel-level changes, stores the lease, advances DHCP state, and arms timers.
pub(super) async fn lease<N: Ops>(actor: &mut Actor<N>, index: u32, lease: Lease) -> Result<()> {
    apply(actor, index, &lease).await?;
    store(actor, &lease);
    transition(actor, DhcpState::Bound);
    actor.timers.arm(&lease);

    Ok(())
}

pub(super) fn params<N: Ops>(actor: &Actor<N>) -> Result<([u8; 6], Ipv4Addr, Ipv4Addr)> {
    let lease = actor
        .snapshot
        .lease
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", actor.snapshot.name))?;

    Ok((actor.snapshot.mac, lease.server_ip, lease.assigned_ip))
}
