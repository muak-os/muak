//! Bridge provisioning logic for a per-interface actor.

use anyhow::{Context as _, Result};
use netdev::Ops;
use netlink::link::ifname::Name;
use netlink::link::state::State;

use super::Actor;
use crate::interface::lifecycle::Lifecycle;
use crate::interface::snapshot::Snapshot;

/// Creates the bridge device that owns this port and returns the bridge snapshot.
pub(super) async fn configure<N: Ops>(
    actor: &mut Actor<N>,
    bridge_name: &str,
    stp: bool,
) -> Result<Snapshot> {
    kmsg::info!(
        "Setting up bridge {} with port {}",
        bridge_name,
        actor.snapshot.name
    );
    let mac = actor
        .ops
        .ensure_bridge(bridge_name, actor.snapshot.name.as_str(), stp)
        .await?;
    kmsg::info!(
        "Bridge setup complete: {} <- {}",
        bridge_name,
        actor.snapshot.name
    );

    actor.timers.disarm();

    let index = actor.ops.index(bridge_name).await?;
    let br_iface_name =
        Name::new(bridge_name).with_context(|| format!("invalid bridge name: {bridge_name}"))?;

    let bridge_snapshot = Snapshot {
        name: br_iface_name.clone(),
        state: Lifecycle::Discovered,
        index,
        mac,
        link: State::Up,
        ip: None,
        lease: None,
        dhcp_state: None,
        ipv6: None,
        l3_owner: br_iface_name,
    };

    let snapshot = &mut actor.snapshot;
    snapshot.ip = None;
    snapshot.lease = None;
    snapshot.dhcp_state = None;
    snapshot.l3_owner = bridge_snapshot.name.clone();
    actor.publish_snapshot();

    Ok(bridge_snapshot)
}
