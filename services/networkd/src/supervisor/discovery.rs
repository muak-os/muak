//! Ethernet interface discovery and carrier-aware selection at startup.

use core::time::Duration;

use anyhow::{Result, bail};
use netlib::interface::{Ethernet, Selector};
use netlib::link::State;
use netlib::netlink::Ops;

use super::NetworkSupervisor;
use crate::interface::snapshot::Snapshot;
use crate::interface::state::Lifecycle;
use crate::supervisor::state::NetworkState;

/// Timeout for carrier detection when probing interfaces.
const CARRIER_TIMEOUT_SECS: u64 = 6;

impl<N: Ops> NetworkSupervisor<N> {
    pub(super) async fn discover_interfaces(&mut self) -> Result<()> {
        kmsg::info!("Discovering ethernet interfaces");
        self.state.transition(NetworkState::Initializing)?;
        self.publish_state();

        let mut discovered = self.ops.discover_ethernet().await?;
        if discovered.is_empty() {
            self.state.transition(NetworkState::Degraded)?;
            self.publish_state();
            bail!("no ethernet interfaces found");
        }

        let timeout = Duration::from_secs(CARRIER_TIMEOUT_SECS);
        let carrier_states = self.probe_all_for_carrier(&discovered, timeout).await;

        let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
        if !any_carrier {
            self.state.transition(NetworkState::Degraded)?;
            self.publish_state();
            bail!(
                "no carrier detected on any interface after {CARRIER_TIMEOUT_SECS}s - check cable connections"
            );
        }

        for iface in &mut discovered {
            iface.link_state = carrier_link_state(carrier_states.get(&iface.index));
        }

        self.spawn_interface_actors(&discovered);
        self.select_primary_interface(&discovered)?;

        self.state.transition(NetworkState::Operational)?;
        self.sync_and_publish();
        kmsg::info!(
            "Discovered {} interfaces, primary={:?}",
            discovered.len(),
            self.state.primary
        );

        Ok(())
    }

    async fn probe_all_for_carrier(
        &self,
        interfaces: &[Ethernet],
        timeout: Duration,
    ) -> std::collections::HashMap<u32, bool> {
        let pairs: Vec<(u32, &str)> = interfaces
            .iter()
            .map(|i| (i.index, i.name.as_str()))
            .collect();

        self.ops.probe_carriers(&pairs, timeout).await
    }

    fn spawn_interface_actors(&mut self, discovered: &[Ethernet]) {
        for iface in discovered {
            let snapshot = Snapshot {
                name: iface.name.clone(),
                state: Lifecycle::Discovered,
                index: iface.index,
                mac: iface.mac_address,
                link: iface.link_state.clone(),
                ip: None,
                lease: None,
                dhcp_state: None,
                ipv6: None,
                l3_owner: iface.name.clone(),
            };
            self.spawn_interface_actor(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[Ethernet]) -> Result<()> {
        let primary = Selector::select_primary(discovered)
            .ok_or_else(|| anyhow::anyhow!("select_primary_interface called with empty list"))?;

        self.state.primary = Some(primary.name.clone());

        let backups = Selector::select_backups(discovered, &primary.name);
        self.state.backups = backups.iter().map(|i| i.name.clone()).collect();

        kmsg::info!(
            "Selected primary: {} (state: {}, carrier: {}), backups: {:?}",
            primary.name,
            primary.link_state,
            if primary.has_carrier() { "yes" } else { "no" },
            self.state.backups
        );
        Ok(())
    }
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
