//! IPv4 and IPv6 address management.

use core::future::Future;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use rtnetlink::Handle;
use rtnetlink::packet_route::address::AddressAttribute;
use thiserror::Error;
use tokio_stream::StreamExt as _;

use crate::netlink::Rtnl;

/// Address management failure conditions.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to add IPv4 address.
    #[error("failed to add IPv4 address: {0}")]
    AddIpv4(#[source] rtnetlink::Error),
    /// Failed to remove IPv4 address.
    #[error("failed to remove IPv4 address: {0}")]
    RemoveIpv4(#[source] rtnetlink::Error),
    /// Failed to add IPv6 address.
    #[error("failed to add IPv6 address: {0}")]
    AddIpv6(#[source] rtnetlink::Error),
    /// Failed to remove IPv6 address.
    #[error("failed to remove IPv6 address: {0}")]
    RemoveIpv6(#[source] rtnetlink::Error),
    /// Failed to enumerate addresses.
    #[error("failed to enumerate addresses: {0}")]
    List(#[source] rtnetlink::Error),
}

/// Address management result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// IPv4 address configuration acquired via DHCP or static assignment.
#[derive(Debug, Clone)]
pub struct IpConfig {
    /// IPv4 address.
    pub address: Ipv4Addr,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Optional default gateway.
    pub gateway: Option<Ipv4Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv4Addr>,
}

/// IPv6 address configuration acquired via SLAAC or static assignment.
#[derive(Debug, Clone)]
pub struct Ipv6Config {
    /// IPv6 address.
    pub address: Ipv6Addr,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Optional default gateway.
    pub gateway: Option<Ipv6Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv6Addr>,
}

/// Trait covering all address-layer netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Adds or confirms an IPv4 address on an interface.
    fn ensure_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Returns the first IPv4 address on an interface, if any.
    fn find_ipv4(&self, index: u32) -> impl Future<Output = Result<Option<(Ipv4Addr, u8)>>> + Send;

    /// Returns whether an interface has at least one IPv4 address.
    fn has_ipv4(&self, index: u32) -> impl Future<Output = Result<bool>> + Send;

    /// Adds an IPv4 address to an interface.
    fn add_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Removes an IPv4 address from an interface.
    fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> impl Future<Output = Result<()>> + Send;

    /// Adds or confirms an IPv6 address on an interface.
    fn ensure_ipv6(
        &self,
        index: u32,
        ip: Ipv6Addr,
        prefix: u8,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Removes an IPv6 address from an interface.
    fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> impl Future<Output = Result<()>> + Send;
}

impl Ops for Rtnl {
    async fn ensure_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
        ensure_ipv4(&self.handle, index, ip, prefix).await
    }

    async fn find_ipv4(&self, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
        find_ipv4(&self.handle, index).await
    }

    async fn has_ipv4(&self, index: u32) -> Result<bool> {
        has_ipv4(&self.handle, index).await
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
        add_ipv4(&self.handle, index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> Result<()> {
        remove_ipv4(&self.handle, index, ip).await
    }

    async fn ensure_ipv6(&self, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
        ensure_ipv6(&self.handle, index, ip, prefix).await
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> Result<()> {
        remove_ipv6(&self.handle, index, ip).await
    }
}

/// Finds the first non-link-local IPv4 address and prefix length on a given interface.
pub(crate) async fn find_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v4) = find_v4_in_attributes(&addr.attributes) {
            return Ok(Some((v4, addr.header.prefix_len)));
        }
    }

    Ok(None)
}

/// Returns true if the interface has at least one IPv4 address assigned.
pub(crate) async fn has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let has_v4 = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V4(_))));
        if has_v4 {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Adds an IPv4 address with prefix length to the given interface.
pub(crate) async fn add_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv4)
}

/// Removes a specific IPv4 address from the given interface (no-op if absent).
pub(crate) async fn remove_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let matches = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V4(v4)) if *v4 == ip));
        if matches {
            handle
                .address()
                .del(addr)
                .execute()
                .await
                .map_err(Failure::RemoveIpv4)?;
            return Ok(());
        }
    }

    Ok(())
}

/// Adds an IPv4 address only if it is not already present with the same prefix.
async fn ensure_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv4(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv4)
}

async fn ensure_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    let mut addrs = handle.address().get().execute();
    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v6) = find_v6_in_attributes(&addr.attributes)
            && v6.segments()[0] != 0xfe80
            && v6 == ip
            && addr.header.prefix_len == prefix
        {
            return Ok(());
        }
    }
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Failure::AddIpv6)
}

async fn remove_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();
    while let Some(addr) = addrs.try_next().await.map_err(Failure::List)? {
        if addr.header.index != index {
            continue;
        }
        let matches = addr
            .attributes
            .iter()
            .any(|attr| matches!(attr, AddressAttribute::Address(IpAddr::V6(v6)) if *v6 == ip));
        if matches {
            handle
                .address()
                .del(addr)
                .execute()
                .await
                .map_err(Failure::RemoveIpv6)?;
            return Ok(());
        }
    }
    Ok(())
}

fn find_v4_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv4Addr> {
    attributes.iter().find_map(|attr| {
        if let &AddressAttribute::Address(IpAddr::V4(v4)) = attr {
            Some(v4)
        } else {
            None
        }
    })
}

fn find_v6_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv6Addr> {
    attributes.iter().find_map(|attr| {
        if let &AddressAttribute::Address(IpAddr::V6(v6)) = attr {
            Some(v6)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_v4_in_attributes_returns_first_ipv4_address() {
        // ARRANGE
        let attributes = vec![
            AddressAttribute::Address(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 11))),
        ];

        // ACT
        let address = find_v4_in_attributes(&attributes);

        // ASSERT
        assert_eq!(address, Some(Ipv4Addr::new(192, 0, 2, 10)));
    }

    #[test]
    fn find_v6_in_attributes_returns_first_ipv6_address() {
        // ARRANGE
        let expected = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let attributes = vec![
            AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
            AddressAttribute::Address(IpAddr::V6(expected)),
        ];

        // ACT
        let address = find_v6_in_attributes(&attributes);

        // ASSERT
        assert_eq!(address, Some(expected));
    }

    #[test]
    fn find_v4_in_attributes_returns_none_without_ipv4() {
        // ARRANGE
        let attributes = vec![AddressAttribute::Address(IpAddr::V6(Ipv6Addr::LOCALHOST))];

        // ACT
        let address = find_v4_in_attributes(&attributes);

        // ASSERT
        assert!(address.is_none());
    }

    #[test]
    fn find_v6_in_attributes_returns_none_without_ipv6() {
        // ARRANGE
        let attributes = vec![AddressAttribute::Address(IpAddr::V4(Ipv4Addr::new(
            192, 0, 2, 10,
        )))];

        // ACT
        let address = find_v6_in_attributes(&attributes);

        // ASSERT
        assert!(address.is_none());
    }
}
