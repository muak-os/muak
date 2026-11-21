//! Ethernet interface discovery and carrier-aware selection at startup.

use core::time::Duration;

use anyhow::{Result, bail};
use netdev::Ops;
use netdev::interface::{Ethernet, Selector};
use netlink::link::state::State;

use super::{NetworkSupervisor, dns};
use crate::supervisor::NetworkState;

/// Timeout for carrier detection when probing interfaces.
const CARRIER_TIMEOUT_SECS: u64 = 6;

/// Discovers Ethernet interfaces and spawns actors for each of them.
pub(super) async fn interfaces<N: Ops>(supervisor: &mut NetworkSupervisor<N>) -> Result<()> {
    kmsg::info!("Discovering ethernet interfaces");
    supervisor.state.transition(NetworkState::Initializing)?;

    let mut discovered = supervisor.ops.discover_ethernet().await?;
    if discovered.is_empty() {
        supervisor.state.transition(NetworkState::Degraded)?;
        bail!("no ethernet interfaces found");
    }

    let pairs: Vec<(u32, &str)> = discovered
        .iter()
        .map(|iface| (iface.index, iface.name.as_str()))
        .collect();
    let carrier_states = supervisor
        .ops
        .probe_carriers(&pairs, Duration::from_secs(CARRIER_TIMEOUT_SECS))
        .await?;

    let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
    if !any_carrier {
        supervisor.state.transition(NetworkState::Degraded)?;
        bail!(
            "no carrier detected on any interface after {CARRIER_TIMEOUT_SECS}s - check cable connections"
        );
    }

    for iface in &mut discovered {
        iface.link_state = carrier_link_state(carrier_states.get(&iface.index));
    }

    spawn_interfaces(supervisor, &discovered);
    select_primary_interface(supervisor, &discovered)?;

    supervisor.state.transition(NetworkState::Operational)?;
    dns::flush(supervisor);
    kmsg::info!(
        "Discovered {} interfaces, primary={:?}",
        discovered.len(),
        supervisor.state.primary
    );

    Ok(())
}

fn spawn_interfaces<N: Ops>(supervisor: &mut NetworkSupervisor<N>, discovered: &[Ethernet]) {
    for iface in discovered {
        supervisor.spawn_interface(iface.clone());
    }
}

fn select_primary_interface<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    discovered: &[Ethernet],
) -> Result<()> {
    let primary = Selector::select_primary(discovered)
        .ok_or_else(|| anyhow::anyhow!("select_primary_interface called with empty list"))?;

    supervisor.state.primary = Some(primary.name.clone());

    let backups = Selector::select_backups(discovered, &primary.name);
    supervisor.state.backups = backups.iter().map(|i| i.name.clone()).collect();

    kmsg::info!(
        "Selected primary: {} (state: {}, carrier: {}), backups: {:?}",
        primary.name,
        primary.link_state,
        if primary.has_carrier() { "yes" } else { "no" },
        supervisor.state.backups
    );

    Ok(())
}

fn carrier_link_state(has_carrier: Option<&bool>) -> State {
    if has_carrier == Some(&true) {
        State::Up
    } else {
        State::NoCarrier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrier_link_state_true_returns_up() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(Some(&true)), State::Up);
    }

    #[test]
    fn carrier_link_state_false_returns_no_carrier() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(Some(&false)), State::NoCarrier);
    }

    #[test]
    fn carrier_link_state_none_returns_no_carrier() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(None), State::NoCarrier);
    }
}
