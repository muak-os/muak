//! Bridges the SLAAC manager with the network actor for IPv6 autoconfiguration.

use netlib::address::Ipv6Config;
use netlib::{address, route};
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;
use crate::slaac::{SlaacEvent, SlaacManager};

impl NetworkActor {
    /// Starts a SLAAC manager on the given interface, forwarding events to the actor command loop.
    pub(super) async fn try_acquire_slaac(
        &mut self,
        iface_name: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) {
        let mac = match self.get_interface_mac(iface_name) {
            Ok(m) => m,
            Err(_) => {
                kmsg::warn!("Cannot start SLAAC on {}: MAC not found", iface_name);
                return;
            }
        };

        kmsg::info!("Starting SLAAC on {}", iface_name);

        let (slaac_tx, slaac_rx) = mpsc::channel::<SlaacEvent>(16);

        match SlaacManager::new(iface_name.to_string(), mac, slaac_tx) {
            Ok(manager) => {
                tokio::spawn(manager.run());
                Self::forward_slaac_events(slaac_rx, cmd_tx.clone());
                kmsg::info!("SLAAC manager started on {}", iface_name);
            }
            Err(e) => {
                kmsg::info!(
                    "SLAAC not available on {}: {} (continuing with IPv4)",
                    iface_name,
                    e
                );
            }
        }
    }

    fn forward_slaac_events(
        slaac_rx: mpsc::Receiver<SlaacEvent>,
        cmd_tx: mpsc::Sender<NetworkCommand>,
    ) {
        tokio::spawn(relay_slaac_events(slaac_rx, cmd_tx));
    }

    pub(super) async fn handle_slaac_event(&mut self, event: SlaacEvent) {
        match event {
            SlaacEvent::Configured {
                address,
                prefix_len,
                gateway,
                dns,
            } => {
                self.on_slaac_configured(address, prefix_len, gateway, dns)
                    .await;
            }
            SlaacEvent::AddressDeprecated { address } => {
                kmsg::info!("IPv6 address deprecated: {}", address);
            }
            SlaacEvent::AddressExpired { address } => {
                self.on_slaac_address_expired(address).await;
            }
            SlaacEvent::RouterExpired { router } => {
                self.on_slaac_router_expired(router).await;
            }
            SlaacEvent::DnsUpdated { servers } => {
                self.on_slaac_dns_updated(servers);
            }
            SlaacEvent::Failed { reason } => {
                kmsg::warn!("SLAAC failed: {} (continuing with IPv4)", reason);
                self.state.ipv6 = false;
            }
        }
    }

    async fn on_slaac_configured(
        &mut self,
        address: std::net::Ipv6Addr,
        prefix_len: u8,
        gateway: std::net::Ipv6Addr,
        dns: Vec<std::net::Ipv6Addr>,
    ) {
        kmsg::info!("SLAAC configured: {} via {}", address, gateway);

        let primary = match self.get_primary_name() {
            Ok(p) => p,
            Err(_) => return,
        };
        let index = match self.get_interface(primary.as_str()) {
            Some(i) => i.index,
            None => return,
        };

        let ipv6 = Ipv6Config {
            address,
            prefix_len,
            gateway: Some(gateway),
            dns,
        };

        if let Err(e) = self.apply_ipv6_configuration(index, &ipv6).await {
            kmsg::warn!("Failed to apply IPv6 configuration: {}", e);
            return;
        }
        if let Err(e) = self.update_interface_with_ipv6(primary.as_str(), ipv6) {
            kmsg::warn!("Failed to update interface with IPv6: {}", e);
            return;
        }

        self.state.ipv6 = true;
        self.sync_and_publish();
    }

    async fn on_slaac_address_expired(&mut self, address: std::net::Ipv6Addr) {
        kmsg::info!("IPv6 address expired: {}", address);

        let primary = match self.get_primary_name() {
            Ok(p) => p,
            Err(_) => return,
        };
        let index = match self.get_interface(primary.as_str()) {
            Some(i) => i.index,
            None => return,
        };

        if let Err(e) = address::remove_ipv6(&self.handle, index, address).await {
            kmsg::warn!("Failed to remove expired IPv6 address: {}", e);
        }
        if let Some(iface) = self.get_interface_mut(primary.as_str()) {
            iface.ipv6 = None;
        }

        self.state.ipv6 = false;
        self.sync_and_publish();
    }

    fn on_slaac_dns_updated(&mut self, servers: Vec<std::net::Ipv6Addr>) {
        kmsg::info!("IPv6 DNS updated: {} servers", servers.len());

        if let Err(e) = self.update_dns_v6(servers.clone()) {
            kmsg::warn!("Failed to update IPv6 DNS: {}", e);
        }

        let primary = match self.get_primary_name() {
            Ok(p) => p,
            Err(_) => return,
        };
        if let Some(iface) = self.get_interface_mut(primary.as_str())
            && let Some(ipv6) = &mut iface.ipv6
        {
            ipv6.dns = servers;
        }

        self.sync_and_publish();
    }

    pub(super) async fn apply_ipv6_configuration(
        &mut self,
        index: u32,
        ipv6: &Ipv6Config,
    ) -> anyhow::Result<()> {
        address::ensure_ipv6(&self.handle, index, ipv6.address, ipv6.prefix_len).await?;

        if let Some(gateway) = ipv6.gateway {
            kmsg::info!("Setting IPv6 default route via {}", gateway);
            route::ensure_default_route_v6(&self.handle, gateway).await?;
        }

        if !ipv6.dns.is_empty() {
            kmsg::info!("Configuring {} IPv6 DNS server(s)", ipv6.dns.len());
            self.update_dns_v6(ipv6.dns.clone())?;
        }

        Ok(())
    }

    pub(super) fn update_interface_with_ipv6(
        &mut self,
        iface: &str,
        ipv6: Ipv6Config,
    ) -> anyhow::Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ipv6 = Some(ipv6);
        self.sync_and_publish();

        Ok(())
    }

    async fn on_slaac_router_expired(&mut self, router: std::net::Ipv6Addr) {
        kmsg::info!("IPv6 router expired: {}", router);
        if let Err(e) = route::remove_default_route_v6(&self.handle, router).await {
            kmsg::warn!("Failed to remove IPv6 default route: {}", e);
        }
    }
}

async fn relay_slaac_events(
    mut rx: mpsc::Receiver<SlaacEvent>,
    cmd_tx: mpsc::Sender<NetworkCommand>,
) {
    while let Some(event) = rx.recv().await {
        if cmd_tx.send(NetworkCommand::Slaac(event)).await.is_err() {
            break;
        }
    }
}
