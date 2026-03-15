use anyhow::Result;

use super::state::NetworkActor;
use crate::dns::{configure_dns, configure_dns_v6};
use crate::model::{IpConfig, Ipv6Config};
use crate::netlink::{address, route};

impl NetworkActor {
    pub(super) async fn apply_static_ipv4(
        &mut self,
        iface_name: &str,
        index: u32,
        addresses: &[config::Cidr4],
        gateway: Option<std::net::Ipv4Addr>,
    ) -> Result<()> {
        for cidr in addresses {
            address::ensure_ipv4(&self.handle, index, cidr.address, cidr.prefix).await?;
        }

        if let Some(gw) = gateway {
            kmsg::info!("Setting default route via {} on {}", gw, iface_name);
            route::ensure_default_route(&self.handle, gw).await?;
        }

        let dns = config::network().ipv4_dns();
        if !dns.is_empty() {
            configure_dns(&dns)?;
        }

        let primary_addr = addresses.first().expect("addresses is non-empty");
        let ip = IpConfig {
            address: primary_addr.address,
            prefix_len: primary_addr.prefix,
            gateway,
            dns,
        };

        let iface_snap = self
            .get_interface_mut(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?;
        iface_snap.ip = Some(ip);
        self.sync_and_publish();

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
        iface_name: &str,
        index: u32,
        addresses: &[config::Cidr6],
        gateway: Option<std::net::Ipv6Addr>,
    ) -> Result<()> {
        for cidr in addresses {
            address::ensure_ipv6(&self.handle, index, cidr.address, cidr.prefix).await?;
        }

        if let Some(gw) = gateway {
            kmsg::info!("Setting IPv6 default route via {} on {}", gw, iface_name);
            route::ensure_default_route_v6(&self.handle, gw).await?;
        }

        let dns = config::network().ipv6_dns();
        if !dns.is_empty() {
            configure_dns_v6(&dns)?;
        }

        let primary_addr = addresses.first().expect("addresses is non-empty");
        let ipv6 = Ipv6Config {
            address: primary_addr.address,
            prefix_len: primary_addr.prefix,
            gateway,
            dns,
        };

        let iface_snap = self
            .get_interface_mut(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?;
        iface_snap.ipv6 = Some(ipv6);
        self.state.ipv6 = true;
        self.sync_and_publish();

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
