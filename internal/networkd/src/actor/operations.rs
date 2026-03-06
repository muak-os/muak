use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;
use crate::config;
use crate::connectivity::{self, ConnectivityConfig};
use crate::dhcpv4::run_dhcp_client;
use crate::dns::{configure_dns, configure_dns_v6};
use crate::interface::{Interface, InterfaceSelector, LinkState, discover_ethernet_interfaces};
use crate::model::{
    ConnectivityResult, ConnectivityStatus, DhcpLease, InterfaceSnapshot, IpConfig, Ipv6Config,
    LinkStateKind, NetworkStateKind,
};
use crate::netlink::{address, link, route};
use crate::netutil::{bridge, tap};
use crate::slaac::{SlaacEvent, SlaacManager};

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        println!("Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dhcp_on_primary(cmd_tx).await?;

        if sysconfig::network().ipv6 {
            self.try_acquire_slaac_on_primary(cmd_tx).await;
        }

        self.setup_bridge_and_transfer_dhcp(cmd_tx).await?;

        self.state.state = NetworkStateKind::Ready;
        self.publish_state();

        self.start_connectivity_monitoring(cmd_tx.clone());

        println!("Network initialization complete");

        Ok(())
    }

    async fn discover_interfaces(&mut self) -> Result<()> {
        println!("Discovering ethernet interfaces");
        self.state.state = NetworkStateKind::Initializing;
        self.publish_state();

        let mut discovered = discover_ethernet_interfaces(&self.handle).await?;
        if discovered.is_empty() {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            bail!("no ethernet interfaces found");
        }

        let timeout = Duration::from_secs(config::CARRIER_TIMEOUT_SECS);
        let carrier_states = self.probe_all_for_carrier(&discovered, timeout).await;

        let any_carrier = carrier_states.values().any(|&has_carrier| has_carrier);
        if !any_carrier {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            bail!(
                "no carrier detected on any interface after {}s - check cable connections",
                config::CARRIER_TIMEOUT_SECS
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
        println!(
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

        println!(
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

        println!("Acquiring DHCP on primary interface: {}", primary);
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
        let (ip, lease) = run_dhcp_client(iface, &mac).await?;

        self.apply_ip_configuration(index, &ip).await?;
        self.update_interface_with_lease(iface, ip.clone(), lease.clone())?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);

        println!("DHCP acquired on {}: {}", iface, ip.address);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))
    }

    fn get_interface_mac(&self, iface: &str) -> Result<[u8; 6]> {
        self.get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))
    }

    async fn try_acquire_slaac_on_primary(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) {
        let Ok(primary) = self.get_primary_name() else {
            return;
        };

        let Ok(mac) = self.get_interface_mac(&primary) else {
            eprintln!("Cannot start SLAAC: interface MAC not found");
            return;
        };

        println!("Starting SLAAC manager on primary interface: {}", primary);

        let (slaac_tx, mut slaac_rx) = mpsc::channel::<SlaacEvent>(16);

        match SlaacManager::new(primary.clone(), mac, slaac_tx) {
            Ok(manager) => {
                tokio::spawn(manager.run());

                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    while let Some(event) = slaac_rx.recv().await {
                        if cmd_tx.send(NetworkCommand::Slaac(event)).await.is_err() {
                            break;
                        }
                    }
                });

                println!("SLAAC manager started on {}", primary);
            }
            Err(e) => {
                println!("SLAAC not available: {} (continuing with IPv4)", e);
            }
        }
    }

    pub(super) async fn handle_slaac_event(&mut self, event: SlaacEvent) {
        match event {
            SlaacEvent::Configured {
                address,
                prefix_len,
                gateway,
                dns,
            } => {
                println!("SLAAC configured: {} via {}", address, gateway);

                let Ok(primary) = self.get_primary_name() else {
                    return;
                };

                let Some(iface) = self.get_interface(&primary) else {
                    return;
                };

                let index = iface.index;
                let ipv6 = Ipv6Config {
                    address,
                    prefix_len,
                    gateway: Some(gateway),
                    dns,
                };

                if let Err(e) = self.apply_ipv6_configuration(index, &ipv6).await {
                    eprintln!("Failed to apply IPv6 configuration: {}", e);
                    return;
                }

                if let Err(e) = self.update_interface_with_ipv6(&primary, ipv6) {
                    eprintln!("Failed to update interface with IPv6: {}", e);
                    return;
                }

                self.state.ipv6 = true;
                self.sync_and_publish();
            }

            SlaacEvent::AddressDeprecated { address } => {
                println!("IPv6 address deprecated: {}", address);
                // Address is still valid, just shouldn't be used for new connections
                // We log this but don't remove the address yet
            }

            SlaacEvent::AddressExpired { address } => {
                println!("IPv6 address expired: {}", address);

                let Ok(primary) = self.get_primary_name() else {
                    return;
                };

                let Some(iface) = self.get_interface(&primary) else {
                    return;
                };

                let index = iface.index;

                if let Err(e) = address::remove_ipv6(&self.handle, index, address).await {
                    eprintln!("Failed to remove expired IPv6 address: {}", e);
                }

                if let Some(iface) = self.get_interface_mut(&primary) {
                    iface.ipv6 = None;
                }

                self.state.ipv6 = false;
                self.sync_and_publish();
            }

            SlaacEvent::RouterExpired { router } => {
                println!("IPv6 router expired: {}", router);

                if let Err(e) = route::remove_default_route_v6(&self.handle, router).await {
                    eprintln!("Failed to remove IPv6 default route: {}", e);
                }
            }

            SlaacEvent::DnsUpdated { servers } => {
                println!("IPv6 DNS updated: {} servers", servers.len());

                if let Err(e) = configure_dns_v6(&servers) {
                    eprintln!("Failed to update IPv6 DNS: {}", e);
                }

                let Ok(primary) = self.get_primary_name() else {
                    return;
                };

                if let Some(iface) = self.get_interface_mut(&primary)
                    && let Some(ipv6) = &mut iface.ipv6
                {
                    ipv6.dns = servers;
                }

                self.sync_and_publish();
            }

            SlaacEvent::Failed { reason } => {
                println!("SLAAC failed: {} (continuing with IPv4)", reason);
                self.state.ipv6 = false;
            }
        }
    }

    async fn apply_ipv6_configuration(&mut self, index: u32, ipv6: &Ipv6Config) -> Result<()> {
        address::ensure_ipv6(&self.handle, index, ipv6.address, ipv6.prefix_len).await?;

        if let Some(gateway) = ipv6.gateway {
            println!("Setting IPv6 default route via {}", gateway);
            route::ensure_default_route_v6(&self.handle, gateway).await?;
        }

        if !ipv6.dns.is_empty() {
            println!("Configuring {} IPv6 DNS server(s)", ipv6.dns.len());
            configure_dns_v6(&ipv6.dns)?;
        }

        Ok(())
    }

    fn update_interface_with_ipv6(&mut self, iface: &str, ipv6: Ipv6Config) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ipv6 = Some(ipv6);
        self.sync_and_publish();

        Ok(())
    }

    async fn apply_ip_configuration(&mut self, index: u32, ip: &IpConfig) -> Result<()> {
        address::ensure_ipv4(&self.handle, index, ip.address, ip.prefix_len).await?;

        if let Some(gw) = ip.gateway {
            println!("Setting default route via {}", gw);
            route::ensure_default_route(&self.handle, gw).await?;
        } else {
            println!("No gateway in DHCP lease, skipping default route");
        }

        if !ip.dns.is_empty() {
            configure_dns(&ip.dns)?;
        }

        Ok(())
    }

    fn update_interface_with_lease(
        &mut self,
        iface: &str,
        ip: IpConfig,
        lease: DhcpLease,
    ) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ip = Some(ip);
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
            println!("Lease {} attempt for {}", task_name, iface);
            let _ = cmd_tx.send(NetworkCommand::RenewLease { iface }).await;
        })
    }

    pub(super) async fn renew_lease(&mut self, iface: &str) -> Result<()> {
        println!("Renewing DHCP lease for {}", iface);

        let mac = self
            .get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;

        match run_dhcp_client(iface, &mac).await {
            Ok((ip, lease)) => {
                let index = self
                    .get_interface(iface)
                    .ok_or_else(|| anyhow::anyhow!("interface disappeared"))?
                    .index;

                self.apply_ip_configuration(index, &ip).await?;
                self.update_interface_with_lease(iface, ip, lease)?;

                println!("DHCP lease renewed for {}", iface);
                Ok(())
            }
            Err(e) => {
                eprintln!("DHCP renewal failed for {}: {}", iface, e);
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

        println!(
            "Setting up bridge {} with primary {}",
            config::DEFAULT_BRIDGE,
            primary
        );
        bridge::ensure_bridge_with_ip_transfer(
            &self.handle,
            config::DEFAULT_BRIDGE,
            &primary,
            gateway,
        )
        .await?;
        println!(
            "Bridge setup complete: {} <- {}",
            config::DEFAULT_BRIDGE,
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

        let index = link::get_link_index(&self.handle, config::DEFAULT_BRIDGE).await?;
        self.track_bridge_interface(index, mac, lease.clone());
        self.clear_lease_from_primary(&primary);
        self.sync_and_publish();

        println!(
            "Transferring DHCP lease management from {} to {}",
            primary,
            config::DEFAULT_BRIDGE
        );
        self.schedule_lease_renewal(cmd_tx.clone(), config::DEFAULT_BRIDGE.to_string(), lease);

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

        println!(
            "Setting up bridge {} with primary {}",
            config::DEFAULT_BRIDGE,
            primary
        );
        bridge::ensure_bridge_with_ip_transfer(
            &self.handle,
            config::DEFAULT_BRIDGE,
            &primary,
            gateway,
        )
        .await?;
        println!(
            "Bridge setup complete: {} <- {}",
            config::DEFAULT_BRIDGE,
            primary
        );

        Ok(())
    }

    fn track_bridge_interface(&mut self, index: u32, mac: [u8; 6], lease: DhcpLease) {
        let primary = self
            .state
            .primary
            .as_ref()
            .expect("BUG: no primary interface set");
        let ip = self.get_interface(primary).and_then(|i| i.ip.clone());

        let br_snapshot = InterfaceSnapshot {
            name: config::DEFAULT_BRIDGE.to_string(),
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease),
            ipv6: None,
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
        println!("Adding TAP interface: {}", name);

        let index = tap::setup_tap_on_bridge(&self.handle, name, config::DEFAULT_BRIDGE).await?;

        let snapshot = InterfaceSnapshot {
            name: name.to_string(),
            index,
            mac: [0, 0, 0, 0, 0, 0],
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
            ipv6: None,
        };

        self.insert_interface(snapshot.clone());
        self.sync_and_publish();

        println!("TAP interface added: {}", name);
        Ok(snapshot)
    }

    pub(super) async fn delete_tap(&mut self, name: &str) -> Result<()> {
        println!("Deleting TAP interface: {}", name);

        tap::remove_tap_device(&self.handle, name).await?;
        self.remove_interface(name);
        self.sync_and_publish();

        println!("TAP interface deleted: {}", name);
        Ok(())
    }

    fn start_connectivity_monitoring(&mut self, cmd_tx: mpsc::Sender<NetworkCommand>) {
        let interval = std::time::Duration::from_secs(config::CONNECTIVITY_CHECK_INTERVAL_SECS);

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

        let config = ConnectivityConfig::default();
        let result = connectivity::check_connectivity(&config).await;

        self.state.connectivity = result.clone();
        self.publish_state();

        match result.status {
            ConnectivityStatus::Connected if !was_connected => {
                println!("Connectivity OK ({}ms)", result.latency_ms.unwrap_or(0));
            }
            ConnectivityStatus::Disconnected => {
                kmsg::warn!("No internet connectivity detected");
            }
            _ => {}
        }

        result
    }
}
