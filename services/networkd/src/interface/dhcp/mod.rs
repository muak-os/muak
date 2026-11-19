//! DHCP lease life cycle management for a per-interface actor.

pub(super) mod manager;
pub(super) mod refresh;
pub(super) mod timers;

mod commit;

use anyhow::Result;
use dhcp::Scratch;
use dhcp::connector::Connector;
use dhcp::lease::Lease;
use netdev::Ops;

use super::Actor;
use crate::interface::commands::ApplyMode;
use crate::interface::lifecycle::{DhcpState, Lifecycle};

/// Applies DHCP configuration in the selected mode.
pub(super) async fn apply<N: Ops, C: Connector>(
    actor: &mut Actor<N>,
    mode: ApplyMode,
    connector: &C,
) {
    match mode {
        ApplyMode::Provision => start(actor, connector).await,
        ApplyMode::Reconcile => reconcile(actor, connector).await,
    }
}

/// Initialises a `Manager` (binding the socket) and marks the interface as configuring.
pub(super) async fn start<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) {
    actor.set_state(Lifecycle::Configuring);
    let mac = actor.snapshot.mac;
    match manager::Manager::new(actor.snapshot.name.as_str(), mac, connector).await {
        Ok(mgr) => actor.dhcp = Some(mgr),
        Err(e) => {
            kmsg::warn!(
                "Failed to create DHCP socket on {}: {}",
                actor.snapshot.name,
                e
            );
            actor.set_state(Lifecycle::Failed);
        }
    }
}

/// Re-applies DHCP state or restarts acquisition when no lease is cached.
pub(super) async fn reconcile<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) {
    if let Some(lease) = actor.snapshot.lease.clone() {
        let index = actor.snapshot.index;
        if let Err(e) = commit::apply(actor, index, &lease).await {
            kmsg::warn!("DHCP reconcile failed on {}: {}", actor.snapshot.name, e);
            return;
        }

        commit::store(actor, &lease);
        commit::transition(actor, DhcpState::Bound);
        actor.timers.arm(&lease);
        let _configured = actor.ensure_configured_state();
        return;
    }

    if actor.dhcp.is_none() {
        start(actor, connector).await;
    }
}

/// Marks the interface `Failed` once acquisition exhausted its attempts, letting
/// the supervisor consider the primary dead and fail over to a backup.
pub(super) fn acquisition_failed<N: Ops>(actor: &mut Actor<N>) {
    kmsg::warn!("DHCP acquisition gave up on {}", actor.snapshot.name);
    actor.dhcp = None;
    actor.set_state(Lifecycle::Failed);
}

/// Applies a freshly acquired DHCP lease and clears the in-progress manager.
pub(super) async fn acquired<N: Ops>(actor: &mut Actor<N>, lease: Lease) {
    actor.dhcp = None;
    let index = actor.snapshot.index;
    if let Err(e) = commit::lease(actor, index, lease).await {
        kmsg::warn!(
            "Failed to apply DHCP lease on {}: {}",
            actor.snapshot.name,
            e
        );
        actor.set_state(Lifecycle::Failed);
        return;
    }
    actor.set_state(Lifecycle::Configured);
    if let Some(lease) = actor.snapshot.lease.as_ref() {
        kmsg::info!(
            "DHCP acquired on {}: {}",
            actor.snapshot.name,
            lease.assigned_ip
        );
    }
}

/// Re-applies an existing lease after a link-up event without a new DORA exchange.
pub(super) async fn recover_with_lease<N: Ops>(actor: &mut Actor<N>, lease: Lease) {
    let index = actor.snapshot.index;
    if let Err(e) = commit::apply(actor, index, &lease).await {
        kmsg::warn!(
            "Failed to re-apply lease on link-up for {}: {}",
            actor.snapshot.name,
            e
        );
        actor.set_state(Lifecycle::Failed);
        return;
    }
    actor.timers.arm(&lease);
    actor.set_state(Lifecycle::Configured);
}

/// Re-runs a full DORA exchange to recover from a NAK or lease expiry.
pub(super) async fn do_full_dora<N: Ops, C: Connector>(
    actor: &mut Actor<N>,
    connector: &C,
) -> Result<()> {
    kmsg::warn!("DHCP re-acquiring on {}", actor.snapshot.name);
    commit::transition(actor, DhcpState::Init);
    actor.set_state(Lifecycle::Configuring);
    actor.timers.disarm();

    let mac = actor.snapshot.mac;
    let socket = connector.create_raw(actor.snapshot.name.as_str()).await?;
    let mut msg = [0_u8; dhcp::MAX_MESSAGE_LEN];
    let mut frame = [0_u8; dhcp::MAX_FRAME_LEN];
    let lease = dhcp::exchange::acquire(&socket, mac, Scratch::new(&mut msg, &mut frame))
        .await
        .inspect_err(|_| {
            actor.set_state(Lifecycle::Failed);
        })?;

    let index = actor.snapshot.index;
    commit::lease(actor, index, lease).await?;
    actor.set_state(Lifecycle::Configured);

    if let Some(lease) = actor.snapshot.lease.as_ref() {
        kmsg::info!(
            "DHCP re-acquired on {}: {}",
            actor.snapshot.name,
            lease.assigned_ip
        );
    }

    Ok(())
}
