use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use netlink_packet_route::address::AddressAttribute;
use rtnetlink::Handle;
use std::net::{IpAddr, Ipv4Addr};

pub async fn find_ipv4(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
        if addr.header.index == index {
            for attr in &addr.attributes {
                if let AddressAttribute::Address(IpAddr::V4(v4)) = attr {
                    return Ok(Some((*v4, addr.header.prefix_len)));
                }
            }
        }
    }

    Ok(None)
}

pub async fn has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
        if addr.header.index == index {
            for attr in &addr.attributes {
                if matches!(attr, AddressAttribute::Address(IpAddr::V4(_))) {
                    return Ok(true);
                }
            }
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
        if addr.header.index == index {
            for attr in &addr.attributes {
                if let AddressAttribute::Address(IpAddr::V4(v4)) = attr
                    && *v4 == ip
                {
                    handle
                        .address()
                        .del(addr)
                        .execute()
                        .await
                        .context("failed to remove IPv4 address")?;
                    return Ok(());
                }
            }
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
