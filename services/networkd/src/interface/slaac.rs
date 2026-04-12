//! Bridges the SLAAC manager with a per-interface actor for IPv6 autoconfiguration.

use netlib::address::Ipv6Config;
use netlib::{address, route};
use tokio::sync::mpsc;

use super::InterfaceActor;
use super::commands::InterfaceCommand;
use crate::slaac::{SlaacEvent, SlaacManager};

impl InterfaceActor {
    /// Starts a SLAAC manager on this interface, forwarding events to the actor command loop.
    pub(super) async fn try_acquire_slaac(&mut self) {
        let iface_name = self.snapshot.name.to_string();
        let mac = self.snapshot.mac;

        kmsg::info!("Starting SLAAC on {}", iface_name);

        let (slaac_tx, slaac_rx) = mpsc::channel::<SlaacEvent>(16);

        match SlaacManager::new(iface_name.clone(), mac, slaac_tx) {
            Ok(manager) => {
                tokio::spawn(manager.run());
                forward_slaac_events(slaac_rx, self.self_tx.clone());
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
                self.has_ipv6 = false;
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

        let index = self.snapshot.index;
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

        self.snapshot.ipv6 = Some(ipv6);
        self.has_ipv6 = true;
        self.publish_snapshot();
    }

    async fn on_slaac_address_expired(&mut self, address: std::net::Ipv6Addr) {
        kmsg::info!("IPv6 address expired: {}", address);

        let index = self.snapshot.index;
        if let Err(e) = address::remove_ipv6(&self.handle, index, address).await {
            kmsg::warn!("Failed to remove expired IPv6 address: {}", e);
        }
        self.snapshot.ipv6 = None;
        self.has_ipv6 = false;
        self.publish_snapshot();
    }

    fn on_slaac_dns_updated(&mut self, servers: Vec<std::net::Ipv6Addr>) {
        kmsg::info!("IPv6 DNS updated: {} servers", servers.len());

        if let Err(e) = self.update_dns_v6(servers.clone()) {
            kmsg::warn!("Failed to update IPv6 DNS: {}", e);
        }

        if let Some(ipv6) = &mut self.snapshot.ipv6 {
            ipv6.dns = servers;
        }

        self.publish_snapshot();
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

    async fn on_slaac_router_expired(&mut self, router: std::net::Ipv6Addr) {
        kmsg::info!("IPv6 router expired: {}", router);
        if let Err(e) = route::remove_default_route_v6(&self.handle, router).await {
            kmsg::warn!("Failed to remove IPv6 default route: {}", e);
        }
    }
}

fn forward_slaac_events(rx: mpsc::Receiver<SlaacEvent>, cmd_tx: mpsc::Sender<InterfaceCommand>) {
    tokio::spawn(pipe_slaac_to_commands(rx, cmd_tx));
}

async fn pipe_slaac_to_commands(
    mut rx: mpsc::Receiver<SlaacEvent>,
    cmd_tx: mpsc::Sender<InterfaceCommand>,
) {
    while let Some(event) = rx.recv().await {
        let cmd = InterfaceCommand::Slaac(event);
        if cmd_tx.send(cmd).await.is_err() {
            return;
        }
    }
}
