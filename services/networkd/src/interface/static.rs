//! Static IP configuration for a per-interface actor.

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::ops::NetlinkOps;

use super::InterfaceActor;
use crate::interface::state::InterfaceState;

impl<N: NetlinkOps> InterfaceActor<N> {
    /// Applies static IPv4 configuration, logging a warning on failure.
    pub(super) async fn try_apply_static_ipv4(
        &mut self,
        index: u32,
        addresses: &[config::Cidr4],
        gateway: Option<std::net::Ipv4Addr>,
    ) {
        if let Err(e) = self.apply_static_ipv4(index, addresses, gateway).await {
            kmsg::warn!("Static IPv4 failed on {}: {}", self.snapshot.name, e);
        }
    }

    /// Applies static IPv6 configuration, logging a warning on failure.
    pub(super) async fn try_apply_static_ipv6(
        &mut self,
        index: u32,
        addresses: &[config::Cidr6],
        gateway: Option<std::net::Ipv6Addr>,
    ) {
        if let Err(e) = self.apply_static_ipv6(index, addresses, gateway).await {
            kmsg::warn!("Static IPv6 failed on {}: {}", self.snapshot.name, e);
        }
    }

    async fn apply_static_ipv4(
        &mut self,
        index: u32,
        addresses: &[config::Cidr4],
        gateway: Option<std::net::Ipv4Addr>,
    ) -> Result<()> {
        let iface_name = self.snapshot.name.to_string();
        self.set_state(InterfaceState::Configuring);

        for cidr in addresses {
            self.ops
                .ensure_ipv4(index, cidr.address, cidr.prefix)
                .await?;
        }

        if let Some(gw) = gateway {
            kmsg::info!("Setting default route via {} on {}", gw, iface_name);
            self.ops.ensure_default_route(gw).await?;
        }

        let dns = self.config.ipv4_dns();
        if !dns.is_empty() {
            self.dns.update_v4(dns.clone())?;
        }

        let primary_addr = addresses
            .first()
            .ok_or_else(|| anyhow::anyhow!("static IPv4 addresses list is empty"))?;
        let ip = IpConfig {
            address: primary_addr.address,
            prefix_len: primary_addr.prefix,
            gateway,
            dns,
        };

        self.snapshot.ip = Some(ip);
        self.set_state(InterfaceState::Configured);

        kmsg::info!(
            "Static IPv4 configured on {}: {}",
            iface_name,
            addresses
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(())
    }

    pub(super) async fn apply_static_ipv6(
        &mut self,
        index: u32,
        addresses: &[config::Cidr6],
        gateway: Option<std::net::Ipv6Addr>,
    ) -> Result<()> {
        let iface_name = self.snapshot.name.to_string();

        for cidr in addresses {
            self.ops
                .ensure_ipv6(index, cidr.address, cidr.prefix)
                .await?;
        }

        if let Some(gw) = gateway {
            kmsg::info!("Setting IPv6 default route via {} on {}", gw, iface_name);
            self.ops.ensure_default_route_v6(gw).await?;
        }

        let dns = self.config.ipv6_dns();
        if !dns.is_empty() {
            self.dns.update_v6(dns.clone())?;
        }

        let primary_addr = addresses
            .first()
            .ok_or_else(|| anyhow::anyhow!("static IPv6 addresses list is empty"))?;
        let ipv6 = Ipv6Config {
            address: primary_addr.address,
            prefix_len: primary_addr.prefix,
            gateway,
            dns,
        };

        self.snapshot.ipv6 = Some(ipv6);
        self.publish_snapshot();

        kmsg::info!(
            "Static IPv6 configured on {}: {}",
            iface_name,
            addresses
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(())
    }
}
