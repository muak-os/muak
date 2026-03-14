use std::time::Duration;

use anyhow::{Result, bail};
use config::{InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig};
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;
use crate::connectivity::{self, ConnectivityConfig};
use crate::constants;
use crate::interface::{Interface, InterfaceSelector, LinkState, discover_ethernet_interfaces};
use crate::model::{
    ConnectivityResult, ConnectivityStatus, InterfaceSnapshot, LinkStateKind, NetworkStateKind,
};
use crate::netlink::link;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        kmsg::info!("Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dhcp_on_primary(cmd_tx).await?;

        if config::network().ipv6 {
            let primary = self.get_primary_name()?;
            self.try_acquire_slaac(&primary.clone(), cmd_tx).await;
        }

        self.apply_interface_configs(cmd_tx).await?;

        self.state.state = NetworkStateKind::Ready;
        self.publish_state();

        self.start_connectivity_monitoring(cmd_tx.clone());

        kmsg::info!("Network initialization complete");

        Ok(())
    }

    async fn discover_interfaces(&mut self) -> Result<()> {
        kmsg::info!("Discovering ethernet interfaces");
        self.state.state = NetworkStateKind::Initializing;
        self.publish_state();

        let mut discovered = discover_ethernet_interfaces(&self.handle).await?;
        if discovered.is_empty() {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            bail!("no ethernet interfaces found");
        }

        let timeout = Duration::from_secs(constants::CARRIER_TIMEOUT_SECS);
        let carrier_states = self.probe_all_for_carrier(&discovered, timeout).await;

        let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
        if !any_carrier {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            bail!(
                "no carrier detected on any interface after {}s - check cable connections",
                constants::CARRIER_TIMEOUT_SECS
            );
        }

        for iface in &mut discovered {
            iface.link_state = carrier_link_state(carrier_states.get(&iface.index));
        }

        self.populate_interface_map(&discovered);
        self.select_primary_interface(&discovered);

        self.state.state = NetworkStateKind::Operational;
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
            .map(|i| (i.index, i.name.clone()))
            .collect();

        link::probe_interfaces_for_carrier(&self.handle, &pairs, timeout).await
    }

    fn populate_interface_map(&mut self, discovered: &[Interface]) {
        for iface in discovered {
            let snapshot = InterfaceSnapshot {
                name: iface.name.clone(),
                index: iface.index,
                mac: iface.mac_address,
                link: match iface.link_state {
                    LinkState::Up => LinkStateKind::Up,
                    LinkState::NoCarrier | LinkState::Down => LinkStateKind::Down,
                },
                ip: None,
                lease: None,
                ipv6: None,
            };
            self.insert_interface(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[Interface]) {
        let primary = InterfaceSelector::select_primary(discovered)
            .expect("BUG: select_primary_interface called with empty list");

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
    }

    async fn acquire_dhcp_on_primary(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        kmsg::info!("Acquiring DHCP on primary interface: {}", primary);
        self.acquire_dhcp(&primary, cmd_tx).await?;
        Ok(())
    }

    async fn apply_interface_configs(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let interfaces = config::network().interfaces.clone();
        for iface_cfg in &interfaces {
            self.setup_interface_from_config(iface_cfg, cmd_tx).await?;
        }
        Ok(())
    }

    async fn setup_interface_from_config(
        &mut self,
        iface_cfg: &config::InterfaceConfig,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => {
                let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
                self.setup_bridge_from_config(&iface_cfg.name, &bridge_cfg, cmd_tx)
                    .await?;
            }
            InterfaceKind::Ethernet => {
                self.setup_ethernet_from_config(
                    &iface_cfg.name,
                    iface_cfg.ipv4.as_ref(),
                    iface_cfg.ipv6.as_ref(),
                    cmd_tx,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn setup_ethernet_from_config(
        &mut self,
        iface_name: &str,
        ipv4_cfg: Option<&Ipv4InterfaceConfig>,
        ipv6_cfg: Option<&Ipv6InterfaceConfig>,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;

        if iface_name == primary {
            return self
                .override_primary_static_ipv4(iface_name, ipv4_cfg)
                .await;
        }

        if !self.has_interface(iface_name) {
            bail!("ethernet interface '{}' not found", iface_name);
        }

        kmsg::info!("Configuring ethernet interface: {}", iface_name);
        let index = link::ensure_link_up(&self.handle, iface_name).await?;

        match ipv4_cfg {
            Some(ipv4) if ipv4.dhcp => {
                self.acquire_dhcp(iface_name, cmd_tx).await?;
            }
            Some(ipv4) if !ipv4.addresses.is_empty() => {
                self.apply_static_ipv4(iface_name, index, &ipv4.addresses, ipv4.gateway)
                    .await?;
            }
            _ => {}
        }

        if let Some(ipv6) = ipv6_cfg {
            self.configure_ipv6_for_interface(iface_name, index, ipv6, cmd_tx)
                .await?;
        }

        kmsg::info!("Ethernet interface configured: {}", iface_name);
        Ok(())
    }

    async fn override_primary_static_ipv4(
        &mut self,
        iface_name: &str,
        ipv4_cfg: Option<&Ipv4InterfaceConfig>,
    ) -> Result<()> {
        let Some(ipv4) = ipv4_cfg else { return Ok(()) };
        if ipv4.dhcp || ipv4.addresses.is_empty() {
            return Ok(());
        }

        let index = self
            .get_interface(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?
            .index;

        self.cancel_renewal_tasks(iface_name);
        for cidr in &ipv4.addresses {
            crate::netlink::address::remove_ipv4(&self.handle, index, cidr.address)
                .await
                .ok();
        }
        self.apply_static_ipv4(iface_name, index, &ipv4.addresses, ipv4.gateway)
            .await
    }

    async fn configure_ipv6_for_interface(
        &mut self,
        iface_name: &str,
        index: u32,
        ipv6: &Ipv6InterfaceConfig,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        if !ipv6.addresses.is_empty() {
            self.apply_static_ipv6(iface_name, index, &ipv6.addresses, ipv6.gateway)
                .await?;
        } else if ipv6.autoconf && config::network().ipv6 {
            self.try_acquire_slaac(iface_name, cmd_tx).await;
        }
        Ok(())
    }

    pub(super) fn start_connectivity_monitoring(&mut self, cmd_tx: mpsc::Sender<NetworkCommand>) {
        let interval = Duration::from_secs(constants::CONNECTIVITY_CHECK_INTERVAL_SECS);
        let task = tokio::spawn(run_connectivity_monitor(cmd_tx, interval));
        self.connectivity_task = Some(task);
    }

    pub(super) async fn check_connectivity(&mut self) -> ConnectivityResult {
        let was_connected = self.state.connectivity.status == ConnectivityStatus::Connected;
        self.state.connectivity.status = ConnectivityStatus::Checking;
        self.publish_state();

        let cfg = ConnectivityConfig::from_network_config();
        let result = connectivity::check_connectivity(&cfg).await;

        self.state.connectivity = result.clone();
        self.publish_state();

        match result.status {
            ConnectivityStatus::Connected if !was_connected => {
                kmsg::info!("Connectivity OK ({}ms)", result.latency_ms.unwrap_or(0));
            }
            ConnectivityStatus::Disconnected => {
                kmsg::warn!("No internet connectivity detected");
            }
            _ => {}
        }

        result
    }

    pub(super) fn get_primary_name(&self) -> Result<String> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    pub(super) fn extract_lease_mac_and_gateway(
        &self,
        iface_name: &str,
    ) -> Result<(crate::model::DhcpLease, [u8; 6], Option<std::net::Ipv4Addr>)> {
        let iface = self
            .get_interface(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?;

        let lease = iface
            .lease
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", iface_name))?;

        let gateway = iface.ip.as_ref().and_then(|ip| ip.gateway);

        Ok((lease, iface.mac, gateway))
    }
}

fn carrier_link_state(has_carrier: Option<&bool>) -> LinkState {
    if has_carrier == Some(&true) {
        LinkState::Up
    } else {
        LinkState::NoCarrier
    }
}

async fn run_connectivity_monitor(
    cmd_tx: tokio::sync::mpsc::Sender<NetworkCommand>,
    interval: std::time::Duration,
) {
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now(), interval);
    loop {
        timer.tick().await;
        if cmd_tx
            .send(NetworkCommand::PeriodicConnectivityCheck)
            .await
            .is_err()
        {
            break;
        }
    }
}
