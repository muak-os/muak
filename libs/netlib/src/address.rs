//! IPv4 and IPv6 address management.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use rtnetlink::Handle;
use rtnetlink::packet_route::address::AddressAttribute;
use thiserror::Error;
use tokio_stream::StreamExt;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to add IPv4 address: {0}")]
    AddIpv4(#[source] rtnetlink::Error),
    #[error("failed to remove IPv4 address: {0}")]
    RemoveIpv4(#[source] rtnetlink::Error),
    #[error("failed to add IPv6 address: {0}")]
    AddIpv6(#[source] rtnetlink::Error),
    #[error("failed to remove IPv6 address: {0}")]
    RemoveIpv6(#[source] rtnetlink::Error),
    #[error("failed to enumerate addresses: {0}")]
    List(#[source] rtnetlink::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Finds the first non-link-local IPv4 address and prefix length on a given interface.
pub async fn find_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Error::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v4) = find_v4_in_attributes(&addr.attributes) {
            return Ok(Some((v4, addr.header.prefix_len)));
        }
    }

    Ok(None)
}

fn find_v4_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv4Addr> {
    attributes.iter().find_map(|attr| match attr {
        AddressAttribute::Address(IpAddr::V4(v4)) => Some(*v4),
        _ => None,
    })
}

/// Returns true if the interface has at least one IPv4 address assigned.
pub async fn has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Error::List)? {
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
pub async fn add_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Error::AddIpv4)
}

/// Removes a specific IPv4 address from the given interface (no-op if absent).
pub async fn remove_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Error::List)? {
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
                .map_err(Error::RemoveIpv4)?;
            return Ok(());
        }
    }

    Ok(())
}

/// Ensures an IPv4 address with the given prefix is present on the interface.
pub async fn ensure_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv4(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }

    add_ipv4(handle, index, ip, prefix).await
}

/// Finds the first non-link-local IPv6 address and prefix length on a given interface.
pub async fn find_ipv6(handle: &Handle, index: u32) -> Result<Option<(Ipv6Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Error::List)? {
        if addr.header.index != index {
            continue;
        }
        if let Some(v6) = find_v6_in_attributes(&addr.attributes)
            && v6.segments()[0] != 0xfe80
        {
            return Ok(Some((v6, addr.header.prefix_len)));
        }
    }

    Ok(None)
}

fn find_v6_in_attributes(attributes: &[AddressAttribute]) -> Option<Ipv6Addr> {
    attributes.iter().find_map(|attr| match attr {
        AddressAttribute::Address(IpAddr::V6(v6)) => Some(*v6),
        _ => None,
    })
}

/// Adds an IPv6 address with prefix length to the given interface.
pub async fn add_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .map_err(Error::AddIpv6)
}

/// Ensures an IPv6 address with the given prefix is present on the interface.
pub async fn ensure_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv6(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }

    add_ipv6(handle, index, ip, prefix).await
}

/// Removes a specific IPv6 address from the given interface (no-op if absent).
pub async fn remove_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await.map_err(Error::List)? {
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
                .map_err(Error::RemoveIpv6)?;
            return Ok(());
        }
    }

    Ok(())
}

/// IPv4 address configuration acquired via DHCP or static assignment.
#[derive(Debug, Clone)]
pub struct IpConfig {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
}

/// IPv6 address configuration acquired via SLAAC or static assignment.
#[derive(Debug, Clone)]
pub struct Ipv6Config {
    pub address: Ipv6Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv6Addr>,
    pub dns: Vec<Ipv6Addr>,
}
