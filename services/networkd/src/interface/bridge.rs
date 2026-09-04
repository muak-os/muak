//! Bridge provisioning logic for a per-interface actor.

use anyhow::{Context as _, Result};
use netlib::interface::Name;
use netlib::link::State;
use netlib::netlink::Ops;

use super::Actor;
use crate::dhcp::State as DhcpState;
use crate::interface::snapshot::Snapshot;
use crate::interface::state::Lifecycle;

/// Creates the bridge device, transfers the IP from this port, and returns the bridge snapshot.
pub(super) async fn configure<N: Ops>(
    actor: &mut Actor<N>,
    bridge_name: &str,
    stp: bool,
) -> Result<Snapshot> {
    let lease = actor
        .snapshot
        .lease
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", actor.snapshot.name))?;
    let mac = actor.snapshot.mac;
    let gateway = actor.snapshot.ip.as_ref().and_then(|ip| ip.gateway);

    kmsg::info!(
        "Setting up bridge {} with port {}",
        bridge_name,
        actor.snapshot.name
    );
    actor
        .ops
        .ensure_bridge(bridge_name, actor.snapshot.name.as_str(), gateway, stp)
        .await?;
    kmsg::info!(
        "Bridge setup complete: {} <- {}",
        bridge_name,
        actor.snapshot.name
    );

    actor.timers.disarm();

    let index = actor.ops.index(bridge_name).await?;
    let ip = actor.snapshot.ip.clone();
    let br_iface_name =
        Name::new(bridge_name).with_context(|| format!("invalid bridge name: {bridge_name}"))?;

    let bridge_snapshot = Snapshot {
        name: br_iface_name,
        state: Lifecycle::Configured,
        index,
        mac,
        link: State::Up,
        ip,
        lease: Some(lease),
        dhcp_state: Some(DhcpState::Bound),
        ipv6: None,
        l3_owner: Name::new(bridge_name)
            .with_context(|| format!("invalid bridge name: {bridge_name}"))?,
    };

    let snapshot = &mut actor.snapshot;
    snapshot.ip = None;
    snapshot.lease = None;
    snapshot.dhcp_state = None;
    snapshot.l3_owner = bridge_snapshot.name.clone();

    actor.set_state(Lifecycle::Deconfiguring);
    actor.set_state(Lifecycle::Discovered);
    actor.publish_snapshot();

    Ok(bridge_snapshot)
}
