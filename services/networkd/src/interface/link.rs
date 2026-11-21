//! Link-state event handlers for a per-interface actor.

use ::dhcp_client::connector::Connector;
use netdev::Ops;
use netlink::link::state::State;

use super::Actor;
use crate::interface::dhcp;
use crate::interface::lifecycle::Lifecycle;
#[cfg(feature = "ipv6")]
use crate::interface::slaac;

/// Handles a link-up event on this interface.
pub(super) async fn up<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) {
    actor.snapshot.link = State::Up;
    #[cfg(feature = "ipv6")]
    if actor.slaac_enabled {
        slaac::restart(actor).await;
    }
    if !matches!(
        actor.snapshot.state,
        Lifecycle::Degraded | Lifecycle::Failed
    ) {
        actor.publish_snapshot();
        return;
    }
    if let Some(lease) = actor.snapshot.lease.clone() {
        dhcp::recover_with_lease(actor, lease).await;
        actor.publish_snapshot();
        return;
    }
    if matches!(
        actor.snapshot.state,
        Lifecycle::Degraded | Lifecycle::Failed
    ) && !actor.dhcp_enabled
    {
        actor.ensure_configured_state();
        actor.publish_snapshot();
        return;
    }
    if let Err(e) = dhcp::do_full_dora(actor, connector).await {
        kmsg::warn!(
            "DHCP re-acquire failed on link-up for {}: {e}",
            actor.snapshot.name
        );
    }
    actor.publish_snapshot();
}

/// Handles a link-down event on this interface.
pub(super) fn down<N: Ops>(actor: &mut Actor<N>) {
    actor.snapshot.link = State::Down;
    actor.dhcp = None;
    actor.timers.disarm();
    let next = match actor.snapshot.state {
        Lifecycle::Configured => Some(Lifecycle::Degraded),
        Lifecycle::Configuring => Some(Lifecycle::Failed),
        Lifecycle::Discovered
        | Lifecycle::Degraded
        | Lifecycle::Failed
        | Lifecycle::Deconfiguring => None,
    };
    if let Some(next) = next
        && let Err(e) = actor.snapshot.transition(next)
    {
        kmsg::warn!(
            "Interface {} state transition failed on link-down: {}",
            actor.snapshot.name,
            e
        );
    }
    actor.publish_snapshot();
}
