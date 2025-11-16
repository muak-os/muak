use crate::log;
use crate::network::config::{
    BRIDGE_CREATE_RETRIES, BRIDGE_CREATE_RETRY_DELAY_MS, INTERFACE_ENSLAVE_RETRIES,
    INTERFACE_ENSLAVE_RETRY_DELAY_MS,
};
use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use netlink_packet_route::address::AddressAttribute;
use netlink_packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::Handle;
use std::net::Ipv4Addr;

pub async fn ensure_bridge_with_ip_transfer(
    handle: &Handle,
    bridge_name: &str,
    physical_iface: &str,
) -> Result<()> {
    let phys_index = find_interface_index(handle, physical_iface).await?;
    let br_index = ensure_bridge_exists(handle, bridge_name).await?;

    enslave_interface_to_bridge(handle, phys_index, br_index, physical_iface, bridge_name).await?;
    transfer_ip_to_bridge(handle, phys_index, br_index, bridge_name).await?;

    Ok(())
}

pub async fn attach_to_bridge(handle: &Handle, tap_name: &str, bridge_name: &str) -> Result<()> {
    log!(
        "network",
        "Attaching {} to bridge {}",
        tap_name,
        bridge_name
    );

    let tap_index = find_interface_index(handle, tap_name).await?;
    let bridge_index = find_interface_index(handle, bridge_name).await?;

    handle
        .link()
        .set(tap_index)
        .controller(bridge_index)
        .execute()
        .await?;

    log!("network", "{} attached to bridge {}", tap_name, bridge_name);
    Ok(())
}

async fn find_interface_index(handle: &Handle, name: &str) -> Result<u32> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    if let Some(link) = links.try_next().await? {
        Ok(link.header.index)
    } else {
        anyhow::bail!("interface not found: {}", name)
    }
}

async fn ensure_bridge_exists(handle: &Handle, bridge_name: &str) -> Result<u32> {
    if let Some(index) = try_find_bridge(handle, bridge_name).await? {
        bring_link_up(handle, index).await?;
        return Ok(index);
    }

    create_bridge(handle, bridge_name).await
}

async fn try_find_bridge(handle: &Handle, bridge_name: &str) -> Result<Option<u32>> {
    let mut links = handle
        .link()
        .get()
        .match_name(bridge_name.to_string())
        .execute();

    match links.try_next().await {
        Ok(Some(link)) => Ok(Some(link.header.index)),
        Ok(None) | Err(_) => Ok(None),
    }
}

async fn create_bridge(handle: &Handle, bridge_name: &str) -> Result<u32> {
    handle
        .link()
        .add()
        .bridge(bridge_name.to_string())
        .execute()
        .await
        .context("create bridge")?;

    wait_for_bridge_to_appear(handle, bridge_name).await
}

async fn wait_for_bridge_to_appear(handle: &Handle, bridge_name: &str) -> Result<u32> {
    for _ in 0..BRIDGE_CREATE_RETRIES {
        if let Some(index) = try_find_bridge(handle, bridge_name).await? {
            bring_link_up(handle, index).await?;
            return Ok(index);
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            BRIDGE_CREATE_RETRY_DELAY_MS,
        ))
        .await;
    }

    anyhow::bail!("bridge {} creation timeout", bridge_name)
}

async fn bring_link_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(index)
        .up()
        .execute()
        .await
        .context("bring link up")
}

async fn enslave_interface_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    physical_iface: &str,
    bridge_name: &str,
) -> Result<()> {
    handle.link().set(phys_index).down().execute().await.ok();

    for _ in 0..INTERFACE_ENSLAVE_RETRIES {
        if handle
            .link()
            .set(phys_index)
            .controller(br_index)
            .execute()
            .await
            .is_ok()
        {
            handle.link().set(phys_index).up().execute().await.ok();
            log!(
                "network",
                "Ensured {} attached to bridge {}",
                physical_iface,
                bridge_name
            );
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            INTERFACE_ENSLAVE_RETRY_DELAY_MS,
        ))
        .await;
    }

    anyhow::bail!("failed to enslave {} to {}", physical_iface, bridge_name)
}

async fn transfer_ip_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    bridge_name: &str,
) -> Result<()> {
    let phys_ip = find_ipv4_on_interface(handle, phys_index).await?;
    let has_bridge_ip = interface_has_ipv4(handle, br_index).await?;

    if let Some((ip, prefix)) = phys_ip {
        if !has_bridge_ip {
            remove_ip_from_interface(handle, phys_index, ip).await?;
            add_ip_to_interface(handle, br_index, ip, prefix).await?;
            restore_default_gateway(handle).await?;

            log!(
                "network",
                "Moved IP {}/{} to bridge {}",
                ip,
                prefix,
                bridge_name
            );
        }
    }

    Ok(())
}

async fn find_ipv4_on_interface(handle: &Handle, index: u32) -> Result<Option<(Ipv4Addr, u8)>> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
        if addr.header.index == index {
            for attr in &addr.attributes {
                if let AddressAttribute::Address(std::net::IpAddr::V4(v4)) = attr {
                    return Ok(Some((*v4, addr.header.prefix_len)));
                }
            }
        }
    }

    Ok(None)
}

async fn interface_has_ipv4(handle: &Handle, index: u32) -> Result<bool> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
        if addr.header.index == index {
            for attr in &addr.attributes {
                if let AddressAttribute::Address(std::net::IpAddr::V4(_)) = attr {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

async fn remove_ip_from_interface(handle: &Handle, index: u32, ip: Ipv4Addr) -> Result<()> {
    let mut addrs = handle.address().get().execute();

    while let Some(addr) = addrs.try_next().await? {
        if addr.header.index == index {
            for attr in &addr.attributes {
                if let AddressAttribute::Address(std::net::IpAddr::V4(v4)) = attr {
                    if *v4 == ip {
                        handle.address().del(addr).execute().await?;
                        return Ok(());
                    }
                }
            }
        }
    }

    Ok(())
}

async fn add_ip_to_interface(handle: &Handle, index: u32, ip: Ipv4Addr, prefix: u8) -> Result<()> {
    handle
        .address()
        .add(index, ip.into(), prefix)
        .execute()
        .await
        .context("add IP to interface")
}

async fn restore_default_gateway(handle: &Handle) -> Result<()> {
    if let Some(gateway) = find_default_gateway(handle).await? {
        handle
            .route()
            .add()
            .v4()
            .gateway(gateway)
            .execute()
            .await
            .ok();
    }
    Ok(())
}

async fn find_default_gateway(handle: &Handle) -> Result<Option<Ipv4Addr>> {
    let mut routes = handle.route().get(rtnetlink::IpVersion::V4).execute();

    while let Some(route) = routes.try_next().await? {
        for attr in &route.attributes {
            if let RouteAttribute::Gateway(RouteAddress::Inet(gw)) = attr {
                return Ok(Some(*gw));
            }
        }
    }

    Ok(None)
}
