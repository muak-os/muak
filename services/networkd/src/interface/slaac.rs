//! SLAAC event handling for a per-interface actor.

use std::net::Ipv6Addr;
use std::sync::Arc;

use netlib::address::Ipv6Config;
use netlib::ops::NetlinkOps;

use super::InterfaceActor;
use crate::slaac::{SlaacEvent, SlaacManager};

impl<N: NetlinkOps> InterfaceActor<N> {
    /// Initialises a `SlaacManager`.
    pub(super) async fn start_slaac(&mut self) {
        let iface = self.snapshot.name.to_string();
        let mac = self.snapshot.mac;
        match SlaacManager::new(iface, mac, Arc::clone(&self.config)).await {
            Ok(mgr) => {
                kmsg::info!("Starting SLAAC on {}", self.snapshot.name);
                self.slaac = Some(mgr);
            }
            Err(e) => {
                kmsg::info!(
                    "SLAAC unavailable on {}: {} (continuing with IPv4)",
                    self.snapshot.name,
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
                self.slaac = None;
            }
        }
    }

    async fn on_slaac_configured(
        &mut self,
        address: Ipv6Addr,
        prefix_len: u8,
        gateway: Ipv6Addr,
        dns: Vec<Ipv6Addr>,
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
        self.publish_snapshot();
    }

    async fn on_slaac_address_expired(&mut self, address: Ipv6Addr) {
        kmsg::info!("IPv6 address expired: {}", address);

        let index = self.snapshot.index;
        if let Err(e) = self.ops.remove_ipv6(index, address).await {
            kmsg::warn!("Failed to remove expired IPv6 address: {}", e);
        }
        self.snapshot.ipv6 = None;
        self.publish_snapshot();
    }

    fn on_slaac_dns_updated(&mut self, servers: Vec<Ipv6Addr>) {
        kmsg::info!("IPv6 DNS updated: {} servers", servers.len());

        if let Some(ipv6) = self.snapshot.ipv6.as_mut() {
            ipv6.dns = servers;
            self.publish_snapshot();
        }
    }

    pub(super) async fn apply_ipv6_configuration(
        &mut self,
        index: u32,
        ipv6: &Ipv6Config,
    ) -> anyhow::Result<()> {
        self.ops
            .ensure_ipv6(index, ipv6.address, ipv6.prefix_len)
            .await?;

        if let Some(gateway) = ipv6.gateway {
            kmsg::info!("Setting IPv6 default route via {}", gateway);
            self.ops.ensure_default_route_v6(gateway).await?;
        }

        Ok(())
    }

    async fn on_slaac_router_expired(&mut self, router: Ipv6Addr) {
        kmsg::info!("IPv6 router expired: {}", router);
        if let Err(e) = self.ops.remove_default_route_v6(router).await {
            kmsg::warn!("Failed to remove IPv6 default route: {}", e);
        }
    }
}
