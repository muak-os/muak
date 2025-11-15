use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::net::Ipv4Addr;

pub async fn ensure_link_up(handle: &Handle, name: &str) -> Result<u32> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    if let Some(link) = links.try_next().await.context("query link failed")? {
        let index = link.header.index;
        let is_up = link
            .header
            .flags
            .iter()
            .any(|f| matches!(f, netlink_packet_route::link::LinkFlag::Up));
        if !is_up {
            handle
                .link()
                .set(index)
                .up()
                .execute()
                .await
                .context("set link up")?;
        }
        Ok(index)
    } else {
        anyhow::bail!("interface {} not found", name)
    }
}

pub async fn ensure_addr(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    // Scan existing addresses
    let mut addrs = handle.address().get().execute();
    while let Some(a) = addrs.try_next().await? {
        if a.header.index == index {
            for attr in &a.attributes {
                if let netlink_packet_route::address::AddressAttribute::Address(addr) = attr {
                    if let std::net::IpAddr::V4(v4) = addr {
                        if *v4 == ip {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .context("add address")?;
    Ok(())
}

pub async fn ensure_default_route_v4(handle: &Handle, gateway: Ipv4Addr) -> Result<()> {
    // Simple approach: always add; TODO later: inspect existing table to avoid dup.
    handle
        .route()
        .add()
        .v4()
        .gateway(gateway)
        .execute()
        .await
        .context("add default route")?;
    Ok(())
}
