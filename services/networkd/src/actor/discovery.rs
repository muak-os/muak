//! Ethernet interface discovery and carrier-aware selection at startup.

use std::time::Duration;

use anyhow::{Result, bail};
use netlib::interface::{Interface, InterfaceSelector};
use netlib::link::LinkStateKind;

use super::state::{InterfaceSnapshot, NetworkActor, NetworkStateKind};

/// Timeout for carrier detection when probing interfaces.
const CARRIER_TIMEOUT_SECS: u64 = 6;

impl NetworkActor {
    pub(super) async fn discover_interfaces(&mut self) -> Result<()> {
        kmsg::info!("Discovering ethernet interfaces");
        self.state.transition(NetworkStateKind::Initializing)?;
        self.publish_state();

        let mut discovered = netlib::interface::discover_ethernet(&self.handle).await?;
        if discovered.is_empty() {
            self.state.transition(NetworkStateKind::Degraded)?;
            self.publish_state();
            bail!("no ethernet interfaces found");
        }

        let timeout = Duration::from_secs(CARRIER_TIMEOUT_SECS);
        let carrier_states = self.probe_all_for_carrier(&discovered, timeout).await;

        let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
        if !any_carrier {
            self.state.transition(NetworkStateKind::Degraded)?;
            self.publish_state();
            bail!(
                "no carrier detected on any interface after {}s - check cable connections",
                CARRIER_TIMEOUT_SECS
            );
        }

        for iface in &mut discovered {
            iface.link_state = carrier_link_state(carrier_states.get(&iface.index));
        }

        self.populate_interface_map(&discovered);
        self.select_primary_interface(&discovered)?;

        self.state.transition(NetworkStateKind::Operational)?;
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
        interfaces: &[Interface],
        timeout: Duration,
    ) -> std::collections::HashMap<u32, bool> {
        let pairs: Vec<(u32, String)> = interfaces
            .iter()
            .map(|i| (i.index, i.name.to_string()))
            .collect();

        netlib::link::probe_interfaces_for_carrier(&self.handle, &pairs, timeout).await
    }

    fn populate_interface_map(&mut self, discovered: &[Interface]) {
        for iface in discovered {
            let snapshot = InterfaceSnapshot {
                name: iface.name.clone(),
                index: iface.index,
                mac: iface.mac_address,
                link: iface.link_state.clone(),
                ip: None,
                lease: None,
                dhcp_state: None,
                ipv6: None,
            };
            self.insert_interface(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[Interface]) -> Result<()> {
        let primary = InterfaceSelector::select_primary(discovered).ok_or_else(|| {
            anyhow::anyhow!("BUG: select_primary_interface called with empty list")
        })?;

        self.state.primary = Some(primary.name.clone());

        let backups = InterfaceSelector::select_backups(discovered, &primary.name);
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

fn carrier_link_state(has_carrier: Option<&bool>) -> LinkStateKind {
    if has_carrier == Some(&true) {
        LinkStateKind::Up
    } else {
        LinkStateKind::NoCarrier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrier_link_state_true_returns_up() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(Some(&true)), LinkStateKind::Up);
    }

    #[test]
    fn carrier_link_state_false_returns_no_carrier() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(Some(&false)), LinkStateKind::NoCarrier);
    }

    #[test]
    fn carrier_link_state_none_returns_no_carrier() {
        // ACT / ASSERT
        assert_eq!(carrier_link_state(None), LinkStateKind::NoCarrier);
    }
}
