use crate::log;
use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::net::Ipv4Addr;

use anyhow::{Context, Result};

pub async fn ensure_bridge_with_ip_transfer(
    handle: &Handle,
    bridge_name: &str,
    physical_iface: &str,
) -> Result<()> {
    use futures::stream::TryStreamExt;
    use netlink_packet_route::address::AddressAttribute;
    use netlink_packet_route::route::{RouteAddress, RouteAttribute};

    // Find physical index
    let mut phys_links = handle
        .link()
        .get()
        .match_name(physical_iface.to_string())
        .execute();
    let phys_index = if let Some(l) = phys_links.try_next().await.context("query phys link")? {
        l.header.index
    } else {
        anyhow::bail!("physical interface not found: {}", physical_iface);
    };

    // Ensure bridge, with small poll after creation
    let br_index = {
        let mut br_links = handle
            .link()
            .get()
            .match_name(bridge_name.to_string())
            .execute();
        match br_links.try_next().await {
            Ok(Some(l)) => {
                // Bridge already exists
                l.header.index
            }
            Ok(None) | Err(_) => {
                // Bridge doesn't exist or query failed - create it
                handle
                    .link()
                    .add()
                    .bridge(bridge_name.to_string())
                    .execute()
                    .await
                    .context("create bridge")?;
                // Wait up to 30x100ms for the link to appear
                let mut found: Option<u32> = None;
                for _ in 0..30u8 {
                    let mut q = handle
                        .link()
                        .get()
                        .match_name(bridge_name.to_string())
                        .execute();
                    if let Ok(Some(l2)) = q.try_next().await {
                        found = Some(l2.header.index);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                match found {
                    Some(idx) => idx,
                    None => anyhow::bail!("bridge {} creation visible timeout", bridge_name),
                }
            }
        }
    };
    handle
        .link()
        .set(br_index)
        .up()
        .execute()
        .await
        .context("bridge up")?;

    // Enslave physical first (idempotent)
    {
        handle.link().set(phys_index).down().execute().await.ok();
        // If controller set fails with ENODEV due to race, retry a few times
        let mut ok = false;
        for _ in 0..5u8 {
            if handle
                .link()
                .set(phys_index)
                .controller(br_index)
                .execute()
                .await
                .is_ok()
            {
                ok = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !ok {
            anyhow::bail!(
                "failed to enslave {} to {} (ENODEV race?)",
                physical_iface,
                bridge_name
            );
        }
        handle.link().set(phys_index).up().execute().await.ok();
        log!(
            "network",
            "Ensured {} attached to bridge {}",
            physical_iface,
            bridge_name
        );
    }

    // Collect IP on physical & presence on bridge
    let mut phys_ip: Option<(Ipv4Addr, u8)> = None;
    let mut has_bridge_ip = false;
    let mut addrs = handle.address().get().execute();
    while let Some(a) = addrs.try_next().await? {
        if a.header.index == phys_index {
            for attr in &a.attributes {
                if let AddressAttribute::Address(ipaddr) = attr {
                    if let std::net::IpAddr::V4(v4) = ipaddr {
                        phys_ip = Some((*v4, a.header.prefix_len));
                    }
                }
            }
        }
        if a.header.index == br_index {
            for attr in &a.attributes {
                if let AddressAttribute::Address(ipaddr) = attr {
                    if let std::net::IpAddr::V4(_) = ipaddr {
                        has_bridge_ip = true;
                    }
                }
            }
        }
    }

    // Get default gateway presence
    let mut gateway: Option<Ipv4Addr> = None;
    let mut routes = handle.route().get(rtnetlink::IpVersion::V4).execute();
    while let Some(r) = routes.try_next().await? {
        for attr in &r.attributes {
            if let RouteAttribute::Gateway(RouteAddress::Inet(gw)) = attr {
                gateway = Some(*gw);
                break;
            }
        }
    }

    // Move IP if needed
    if let Some((ip, prefix)) = phys_ip {
        if !has_bridge_ip {
            // Delete from physical
            let mut addrs2 = handle.address().get().execute();
            while let Some(a) = addrs2.try_next().await? {
                if a.header.index == phys_index {
                    for attr in &a.attributes {
                        if let AddressAttribute::Address(ipaddr) = attr {
                            if let std::net::IpAddr::V4(v4) = ipaddr {
                                if *v4 == ip {
                                    handle.address().del(a).execute().await?;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // Add to bridge
            handle
                .address()
                .add(br_index, ip.into(), prefix)
                .execute()
                .await?;
            log!(
                "network",
                "Moved IP {}/{} to bridge {}",
                ip,
                prefix,
                bridge_name
            );
            if let Some(gw) = gateway {
                handle.route().add().v4().gateway(gw).execute().await.ok();
            }
        }
    }

    // Enslave physical if not already
    let mut phys_current = handle.link().get().match_index(phys_index).execute();
    if let Some(_link) = phys_current.try_next().await? {
        // Attempt enslave sequence idempotently
        handle.link().set(phys_index).down().execute().await.ok();
        handle
            .link()
            .set(phys_index)
            .controller(br_index)
            .execute()
            .await
            .ok();
        handle.link().set(phys_index).up().execute().await.ok();
        log!(
            "network",
            "Ensured {} attached to bridge {}",
            physical_iface,
            bridge_name
        );
    }

    Ok(())
}

pub async fn attach_to_bridge(handle: &Handle, tap_name: &str, bridge_name: &str) -> Result<()> {
    log!(
        "network",
        "Attaching {} to bridge {}",
        tap_name,
        bridge_name
    );

    let mut tap_links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();
    let tap_index = if let Some(link) = tap_links.try_next().await? {
        link.header.index
    } else {
        anyhow::bail!("TAP device {} not found", tap_name);
    };

    let mut bridge_links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();
    let bridge_index = if let Some(link) = bridge_links.try_next().await? {
        link.header.index
    } else {
        anyhow::bail!("Bridge {} not found", bridge_name);
    };

    handle
        .link()
        .set(tap_index)
        .controller(bridge_index)
        .execute()
        .await?;

    log!("network", "{} attached to bridge {}", tap_name, bridge_name);

    Ok(())
}
