//! Static IP configuration for a per-interface actor.

use core::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::netlink::Ops;

use super::Actor;
use crate::interface::commands::ApplyMode;
use crate::interface::state::Lifecycle;

#[derive(Clone, Copy)]
enum StaticRequest<'a> {
    Ipv4 {
        addresses: &'a [config::Cidr4],
        gateway: Option<Ipv4Addr>,
    },
    Ipv6 {
        addresses: &'a [config::Cidr6],
        gateway: Option<Ipv6Addr>,
    },
}

impl StaticRequest<'_> {
    /// Returns the address family name for diagnostics.
    fn family(self) -> &'static str {
        match self {
            Self::Ipv4 { .. } => "IPv4",
            Self::Ipv6 { .. } => "IPv6",
        }
    }
}

impl<N: Ops> Actor<N> {
    /// Applies static IPv4 configuration in the selected mode.
    pub(super) async fn apply_static_ipv4(
        &mut self,
        index: u32,
        addresses: &[config::Cidr4],
        gateway: Option<Ipv4Addr>,
        mode: ApplyMode,
    ) {
        let request = StaticRequest::Ipv4 { addresses, gateway };
        self.apply_static(index, request, mode).await;
    }

    /// Applies static IPv6 configuration in the selected mode.
    pub(super) async fn apply_static_ipv6(
        &mut self,
        index: u32,
        addresses: &[config::Cidr6],
        gateway: Option<Ipv6Addr>,
        mode: ApplyMode,
    ) {
        let request = StaticRequest::Ipv6 { addresses, gateway };
        self.apply_static(index, request, mode).await;
    }

    /// Applies static configuration in the selected mode.
    async fn apply_static(&mut self, index: u32, request: StaticRequest<'_>, mode: ApplyMode) {
        if let Err(e) = self.set_static(index, request, mode).await {
            kmsg::warn!(
                "Static {} failed on {}: {}",
                request.family(),
                self.snapshot.name,
                e
            );
        }
    }

    /// Applies the desired static state and updates the snapshot.
    async fn set_static(
        &mut self,
        index: u32,
        request: StaticRequest<'_>,
        mode: ApplyMode,
    ) -> Result<()> {
        if mode == ApplyMode::Provision {
            self.set_state(Lifecycle::Configuring);
        }
        self.ensure_static(index, request).await?;
        self.store_static(request)?;
        if mode == ApplyMode::Provision {
            self.set_state(Lifecycle::Configured);
            return Ok(());
        }

        if !self.ensure_configured_state() {
            self.publish_snapshot();
        }
        Ok(())
    }

    /// Ensures every desired static address and route exists in the kernel.
    async fn ensure_static(&self, index: u32, request: StaticRequest<'_>) -> Result<()> {
        match request {
            StaticRequest::Ipv4 { addresses, gateway } => {
                self.ensure_ipv4_addresses(index, addresses).await?;
                self.ensure_ipv4_gateway(gateway).await?;
            }
            StaticRequest::Ipv6 { addresses, gateway } => {
                self.ensure_ipv6_addresses(index, addresses).await?;
                self.ensure_ipv6_gateway(gateway).await?;
            }
        }

        Ok(())
    }

    /// Ensures every desired static IPv4 address exists in the kernel.
    async fn ensure_ipv4_addresses(&self, index: u32, addresses: &[config::Cidr4]) -> Result<()> {
        for cidr in addresses {
            self.ops
                .ensure_ipv4(index, cidr.address, cidr.prefix)
                .await?;
        }

        Ok(())
    }

    /// Ensures the desired static IPv4 default route exists in the kernel.
    async fn ensure_ipv4_gateway(&self, gateway: Option<Ipv4Addr>) -> Result<()> {
        if let Some(gateway) = gateway {
            self.ops.ensure_default_route(gateway).await?;
        }

        Ok(())
    }

    /// Ensures every desired static IPv6 address exists in the kernel.
    async fn ensure_ipv6_addresses(&self, index: u32, addresses: &[config::Cidr6]) -> Result<()> {
        for cidr in addresses {
            self.ops
                .ensure_ipv6(index, cidr.address, cidr.prefix)
                .await?;
        }

        Ok(())
    }

    /// Ensures the desired static IPv6 default route exists in the kernel.
    async fn ensure_ipv6_gateway(&self, gateway: Option<Ipv6Addr>) -> Result<()> {
        if let Some(gateway) = gateway {
            self.ops.ensure_default_route_v6(gateway).await?;
        }

        Ok(())
    }

    /// Stores static address configuration in the interface snapshot.
    fn store_static(&mut self, request: StaticRequest<'_>) -> Result<()> {
        match request {
            StaticRequest::Ipv4 { addresses, gateway } => {
                let primary_addr = first_address(addresses, "IPv4")?;
                let ip = IpConfig {
                    address: primary_addr.address,
                    prefix_len: primary_addr.prefix,
                    gateway,
                    dns: self.config.ipv4_dns().collect(),
                };

                self.snapshot.ip = Some(ip);
            }
            StaticRequest::Ipv6 { addresses, gateway } => {
                let primary_addr = first_address(addresses, "IPv6")?;
                let ipv6 = Ipv6Config {
                    address: primary_addr.address,
                    prefix_len: primary_addr.prefix,
                    gateway,
                    dns: self.config.ipv6_dns().collect(),
                };

                self.snapshot.ipv6 = Some(ipv6);
            }
        }

        Ok(())
    }
}

/// Returns the first configured static address or an error when none were provided.
fn first_address<'a, T>(addresses: &'a [T], family: &str) -> Result<&'a T> {
    addresses
        .first()
        .ok_or_else(|| anyhow::anyhow!("static {family} addresses list is empty"))
}
