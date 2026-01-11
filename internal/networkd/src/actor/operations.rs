use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::connectivity::{self, ConnectivityConfig};
use crate::dhcp::run_dhcp_client;
use crate::dns::configure_dns;
use crate::interface::InterfaceSelector;
use crate::interface::{LinkState, discover_ethernet_interfaces};
use crate::model::{
    ConnectivityResult, ConnectivityStatus, DhcpLease, InterfaceSnapshot, LinkStateKind,
    NetworkStateKind,
};
use crate::netlink::{address, link, route};
use crate::services::{bridge, tap};

use super::commands::NetworkCommand;
use super::state::NetworkActor;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        kmsg::info!(@ "networkd", "Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dhcp_on_primary(cmd_tx).await?;
        self.setup_bridge_and_transfer_dhcp(cmd_tx).await?;

        self.state.state = NetworkStateKind::Ready;
        self.publish_state();

        self.start_connectivity_monitoring(cmd_tx.clone());

        kmsg::info!(@ "networkd", "Network initialization complete");

        Ok(())
    }

    async fn discover_interfaces(&mut self) -> Result<()> {
        kmsg::info!(@ "networkd", "Discovering ethernet interfaces");
        self.state.state = NetworkStateKind::Initializing;
        self.publish_state();

        let mut discovered = discover_ethernet_interfaces(&self.handle).await?;
        if discovered.is_empty() {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            anyhow::bail!("no ethernet interfaces found");
        }

        let timeout = Duration::from_secs(self.config.carrier_timeout);
        let carrier_states = self.probe_all_for_carrier(&discovered, timeout).await;

        let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
        if !any_carrier {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            anyhow::bail!(
                "no carrier detected on any interface after {}s - check cable connections",
                self.config.carrier_timeout
            );
        }

        for iface in &mut discovered {
            if carrier_states.get(&iface.index) == Some(&true) {
                iface.link_state = LinkState::Up;
            } else {
                iface.link_state = LinkState::NoCarrier;
            }
        }

        self.populate_interface_map(&discovered);
        self.select_primary_interface(&discovered);

        self.state.state = NetworkStateKind::Operational;
        self.sync_and_publish();
        kmsg::info!(
            @ "networkd",
            "Discovered {} interfaces, primary={:?}",
            discovered.len(),
            self.state.primary
        );

        Ok(())
    }

    async fn probe_all_for_carrier(
        &self,
        interfaces: &[crate::interface::Interface],
        timeout: Duration,
    ) -> std::collections::HashMap<u32, bool> {
        let pairs: Vec<(u32, String)> = interfaces
            .iter()
            .map(|i| (i.index, i.name.clone()))
            .collect();

        link::probe_interfaces_for_carrier(&self.handle, &pairs, timeout).await
    }

    fn populate_interface_map(&mut self, discovered: &[crate::interface::Interface]) {
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
            };
            self.insert_interface(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[crate::interface::Interface]) {
        let primary = InterfaceSelector::select_primary(discovered)
            .expect("BUG: select_primary_interface called with empty list");

        self.state.primary = Some(primary.name.clone());

        let backups = InterfaceSelector::select_backups(discovered, &primary.name);
        self.state.backups = backups.iter().map(|i| i.name.clone()).collect();

        kmsg::info!(
            @ "networkd",
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

        kmsg::info!(
            @ "networkd",
            "Acquiring DHCP on primary interface: {}",
            primary
        );
        self.acquire_dhcp(&primary, cmd_tx).await?;
        Ok(())
    }

    pub(super) async fn acquire_dhcp(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        let index = link::ensure_link_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;
        let (ip_cfg, lease) = run_dhcp_client(iface, &mac).await?;

        self.apply_ip_configuration(index, &ip_cfg).await?;
        self.update_interface_with_lease(iface, ip_cfg.clone(), lease.clone())?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);

        kmsg::info!(@ "networkd", "DHCP acquired on {}: {}", iface, ip_cfg.address);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))
    }

    fn get_interface_mac(&self, iface: &str) -> Result<[u8; 6]> {
        self.get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))
    }

    async fn apply_ip_configuration(
        &mut self,
        index: u32,
        ip_cfg: &crate::model::IpConfig,
    ) -> Result<()> {
        address::ensure_ipv4(&self.handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;

        if let Some(gw) = ip_cfg.gateway {
            kmsg::info!(@ "networkd", "Setting default route via {}", gw);
            route::ensure_default_route(&self.handle, gw).await?;
        } else {
            kmsg::info!(
                @ "networkd",
                "No gateway in DHCP lease, skipping default route"
            );
        }

        if !ip_cfg.dns.is_empty() {
            configure_dns(&ip_cfg.dns)?;
        }

        Ok(())
    }

    fn update_interface_with_lease(
        &mut self,
        iface: &str,
        ip_cfg: crate::model::IpConfig,
        lease: DhcpLease,
    ) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ip = Some(ip_cfg);
        iface_snap.lease = Some(lease);
        self.sync_and_publish();

        Ok(())
    }

    fn schedule_lease_renewal(
        &mut self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        lease: DhcpLease,
    ) {
        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let renew_deadline_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), renew_deadline, "renewal");
        let rebind_deadline_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), rebind_deadline, "rebind");
        let expiry_deadline_task =
            Self::spawn_renewal_task(cmd_tx, iface.clone(), expiry_deadline, "expiry");

        self.track_renewal_task(iface.clone(), renew_deadline_task);
        self.track_renewal_task(iface.clone(), rebind_deadline_task);
        self.track_renewal_task(iface, expiry_deadline_task);
    }

    fn spawn_renewal_task(
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        deadline: std::time::SystemTime,
        task_type: &str,
    ) -> tokio::task::JoinHandle<()> {
        let task_name = task_type.to_string();
        let now = std::time::SystemTime::now();
        let dur = deadline.duration_since(now).ok();

        tokio::spawn(async move {
            let Some(dur) = dur else { return };
            tokio::time::sleep(dur).await;
            kmsg::info!(@ "networkd", "Lease {} attempt for {}", task_name, iface);
            let _ = cmd_tx.send(NetworkCommand::RenewLease { iface }).await;
        })
    }

    pub(super) async fn renew_lease(&mut self, iface: &str) -> Result<()> {
        kmsg::info!(@ "networkd", "Renewing DHCP lease for {}", iface);

        let mac = self
            .get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;

        match run_dhcp_client(iface, &mac).await {
            Ok((ip_cfg, lease)) => {
                let index = self
                    .get_interface(iface)
                    .ok_or_else(|| anyhow::anyhow!("interface disappeared"))?
                    .index;

                self.apply_ip_configuration(index, &ip_cfg).await?;
                self.update_interface_with_lease(iface, ip_cfg, lease)?;

                kmsg::info!(@ "networkd", "DHCP lease renewed for {}", iface);
                Ok(())
            }
            Err(e) => {
                kmsg::warn!(@ "networkd", "DHCP renewal failed for {}: {}", iface, e);
                Err(anyhow::anyhow!("DHCP renewal failed: {}", e))
            }
        }
    }

    pub(super) async fn setup_bridge(&mut self) -> Result<()> {
        let primary = self.get_primary_name()?;
        let gateway = self
            .get_interface(&primary)
            .and_then(|iface| iface.ip.as_ref())
            .and_then(|ip| ip.gateway);

        let bridge_name = &self.config.bridge;
        kmsg::info!(
            @ "networkd",
            "Setting up bridge {} with primary {}",
            bridge_name,
            primary
        );
        bridge::ensure_bridge_with_ip_transfer(&self.handle, bridge_name, &primary, gateway)
            .await?;
        kmsg::info!(
            @ "networkd",
            "Bridge setup complete: {} <- {}",
            bridge_name,
            primary
        );

        Ok(())
    }

    async fn setup_bridge_and_transfer_dhcp(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let (lease, mac, gateway) = self.extract_lease_mac_and_gateway(&primary)?;

        self.setup_bridge_with_gateway(gateway).await?;

        self.cancel_renewal_tasks(&primary);

        let br_index = self.lookup_bridge_index().await?;
        self.track_bridge_interface(br_index, mac, lease.clone());
        self.clear_lease_from_primary(&primary);
        self.sync_and_publish();

        let bridge_name = self.config.bridge.clone();
        kmsg::info!(
            @ "networkd",
            "Transferring DHCP lease management from {} to {}",
            primary,
            bridge_name
        );
        self.schedule_lease_renewal(cmd_tx.clone(), bridge_name, lease);

        Ok(())
    }

    fn get_primary_name(&self) -> Result<String> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    fn extract_lease_mac_and_gateway(
        &self,
        iface_name: &str,
    ) -> Result<(DhcpLease, [u8; 6], Option<std::net::Ipv4Addr>)> {
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

    async fn setup_bridge_with_gateway(
        &mut self,
        gateway: Option<std::net::Ipv4Addr>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let bridge_name = &self.config.bridge;

        kmsg::info!(
            @ "networkd",
            "Setting up bridge {} with primary {}",
            bridge_name,
            primary
        );
        bridge::ensure_bridge_with_ip_transfer(&self.handle, bridge_name, &primary, gateway)
            .await?;
        kmsg::info!(
            @ "networkd",
            "Bridge setup complete: {} <- {}",
            bridge_name,
            primary
        );

        Ok(())
    }

    async fn lookup_bridge_index(&self) -> Result<u32> {
        link::get_link_index(&self.handle, &self.config.bridge).await
    }

    fn track_bridge_interface(&mut self, index: u32, mac: [u8; 6], lease: DhcpLease) {
        let primary = self
            .state
            .primary
            .as_ref()
            .expect("BUG: no primary interface set");
        let ip = self.get_interface(primary).and_then(|i| i.ip.clone());

        let br_snapshot = InterfaceSnapshot {
            name: self.config.bridge.clone(),
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease),
        };

        self.insert_interface(br_snapshot);
    }

    fn clear_lease_from_primary(&mut self, primary: &str) {
        if let Some(iface) = self.get_interface_mut(primary) {
            iface.ip = None;
            iface.lease = None;
        }
    }

    pub(super) async fn add_tap(&mut self, name: &str) -> Result<InterfaceSnapshot> {
        kmsg::info!(@ "networkd", "Adding TAP interface: {}", name);

        let index = tap::setup_tap_on_bridge(&self.handle, name, &self.config.bridge).await?;

        let snapshot = InterfaceSnapshot {
            name: name.to_string(),
            index,
            mac: [0, 0, 0, 0, 0, 0],
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
        };

        self.insert_interface(snapshot.clone());
        self.sync_and_publish();

        kmsg::info!(@ "networkd", "TAP interface added: {}", name);
        Ok(snapshot)
    }

    pub(super) async fn delete_tap(&mut self, name: &str) -> Result<()> {
        kmsg::info!(@ "networkd", "Deleting TAP interface: {}", name);

        tap::remove_tap_device(&self.handle, name).await?;
        self.remove_interface(name);
        self.sync_and_publish();

        kmsg::info!(@ "networkd", "TAP interface deleted: {}", name);
        Ok(())
    }

    fn start_connectivity_monitoring(&mut self, cmd_tx: mpsc::Sender<NetworkCommand>) {
        let interval = std::time::Duration::from_secs(self.config.check_interval_secs);

        let task = tokio::spawn(async move {
            let mut interval_timer =
                tokio::time::interval_at(tokio::time::Instant::now(), interval);

            while {
                interval_timer.tick().await;
                cmd_tx
                    .send(NetworkCommand::PeriodicConnectivityCheck)
                    .await
                    .is_ok()
            } {}
        });

        self.connectivity_task = Some(task);
    }

    pub(super) async fn check_connectivity(&mut self) -> ConnectivityResult {
        let was_connected = self.state.connectivity.status == ConnectivityStatus::Connected;
        self.state.connectivity.status = ConnectivityStatus::Checking;
        self.publish_state();

        let config = ConnectivityConfig::new(
            self.config.probe_timeout_secs,
            self.config.overall_timeout_secs,
        );
        let result = connectivity::check_connectivity(&config).await;

        self.state.connectivity = result.clone();
        self.publish_state();

        match result.status {
            ConnectivityStatus::Connected if !was_connected => {
                kmsg::info!(
                    @ "networkd",
                    "Connectivity OK ({}ms)",
                    result.latency_ms.unwrap_or(0)
                );
            }
            ConnectivityStatus::Disconnected => {
                kmsg::warn!(@ "networkd", "No internet connectivity detected");
            }
            _ => {}
        }

        result
    }
}
