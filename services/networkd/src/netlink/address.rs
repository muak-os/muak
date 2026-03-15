use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use rtnetlink::Handle;
use rtnetlink::packet_route::address::AddressAttribute;
use tokio_stream::StreamExt;

pub async fn find_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
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

pub async fn has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
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

pub async fn add_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .context("failed to add IPv4 address")
}

pub async fn remove_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
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
                .context("failed to remove IPv4 address")?;
            return Ok(());
        }
    }

    // Address not found - this is not an error
    Ok(())
}

pub async fn ensure_ipv4(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv4(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }

    add_ipv4(handle, index, ip, prefix).await
}

pub async fn find_ipv6(handle: &Handle, index: u32) -> Result<Option<(Ipv6Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
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

pub async fn add_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .context("failed to add IPv6 address")
}

pub async fn ensure_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr, prefix: u8) -> Result<()> {
    if let Some((existing_ip, existing_prefix)) = find_ipv6(handle, index).await?
        && existing_ip == ip
        && existing_prefix == prefix
    {
        return Ok(());
    }

    add_ipv6(handle, index, ip, prefix).await
}

pub async fn remove_ipv6(handle: &Handle, index: u32, ip: Ipv6Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
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
                .context("failed to remove IPv6 address")?;
            return Ok(());
        }
    }

    // Address not found - this is not an error
    Ok(())
}
