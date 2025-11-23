use crate::log;
use crate::network::config::{
    BRIDGE_CREATE_RETRIES, BRIDGE_CREATE_RETRY_DELAY_MS, INTERFACE_ENSLAVE_RETRIES,
    INTERFACE_ENSLAVE_RETRY_DELAY_MS,
};
use crate::network::netlink::{address, link, retry, route};
use anyhow::{Context, Result};
use rtnetlink::{Handle, LinkBridge};

pub async fn ensure_bridge_with_ip_transfer(
    handle: &Handle,
    bridge_name: &str,
    physical_iface: &str,
) -> Result<()> {
    let phys_index = link::get_link_index(handle, physical_iface).await?;
    let br_index = ensure_bridge_exists(handle, bridge_name).await?;

    enslave_interface_to_bridge(handle, phys_index, br_index, physical_iface, bridge_name).await?;
    transfer_ip_to_bridge(handle, phys_index, br_index, bridge_name).await?;

    Ok(())
}

pub async fn attach_to_bridge(handle: &Handle, iface_name: &str, bridge_name: &str) -> Result<()> {
    log!(
        "network",
        "Attaching {} to bridge {}",
        iface_name,
        bridge_name
    );

    let iface_index = link::get_link_index(handle, iface_name).await?;
    let bridge_index = link::get_link_index(handle, bridge_name).await?;

    link::set_link_master(handle, iface_index, bridge_index).await?;

    log!(
        "network",
        "{} attached to bridge {}",
        iface_name,
        bridge_name
    );
    Ok(())
}

pub async fn detach_from_bridge(handle: &Handle, iface_name: &str) -> Result<()> {
    log!("network", "Detaching {} from bridge", iface_name);

    let iface_index = link::get_link_index(handle, iface_name).await?;
    link::unset_link_master(handle, iface_index).await?;

    log!("network", "{} detached from bridge", iface_name);
    Ok(())
}

pub async fn migrate_bridge_to_interface(
    handle: &Handle,
    bridge_name: &str,
    old_iface: &str,
    new_iface: &str,
) -> Result<()> {
    log!(
        "network",
        "Migrating bridge {} from {} to {}",
        bridge_name,
        old_iface,
        new_iface
    );

    detach_from_bridge(handle, old_iface).await?;

    let new_index = link::get_link_index(handle, new_iface).await?;
    let br_index = link::get_link_index(handle, bridge_name).await?;

    link::bring_link_down(handle, new_index).await.ok();
    link::set_link_master(handle, new_index, br_index).await?;
    link::bring_link_up(handle, new_index).await?;

    log!(
        "network",
        "Bridge migration complete: {} now attached to {}",
        bridge_name,
        new_iface
    );
    Ok(())
}


async fn ensure_bridge_exists(handle: &Handle, bridge_name: &str) -> Result<u32> {
    if link::link_exists(handle, bridge_name).await? {
        let index = link::get_link_index(handle, bridge_name).await?;
        link::bring_link_up(handle, index).await?;
        return Ok(index);
    }

    create_bridge(handle, bridge_name).await
}

async fn create_bridge(handle: &Handle, bridge_name: &str) -> Result<u32> {
    handle
        .link()
        .add(LinkBridge::new(bridge_name).build())
        .execute()
        .await
        .context("failed to create bridge")?;

    retry::wait_for_condition(
        || async {
            if link::link_exists(handle, bridge_name).await.ok()? {
                let index = link::get_link_index(handle, bridge_name).await.ok()?;
                link::bring_link_up(handle, index).await.ok()?;
                Some(index)
            } else {
                None
            }
        },
        BRIDGE_CREATE_RETRIES,
        BRIDGE_CREATE_RETRY_DELAY_MS,
        &format!("bridge '{}' creation timeout", bridge_name),
    )
    .await
}

async fn enslave_interface_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    physical_iface: &str,
    bridge_name: &str,
) -> Result<()> {
    link::bring_link_down(handle, phys_index).await.ok();

    retry::retry_operation(
        || async { link::set_link_master(handle, phys_index, br_index).await },
        INTERFACE_ENSLAVE_RETRIES,
        INTERFACE_ENSLAVE_RETRY_DELAY_MS,
        &format!("failed to enslave {} to {}", physical_iface, bridge_name),
    )
    .await?;

    link::bring_link_up(handle, phys_index).await.ok();

    log!(
        "network",
        "Enslaved {} to bridge {}",
        physical_iface,
        bridge_name
    );

    Ok(())
}

async fn transfer_ip_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    bridge_name: &str,
) -> Result<()> {
    let phys_ip = address::find_ipv4(handle, phys_index).await?;
    let has_bridge_ip = address::has_ipv4(handle, br_index).await?;

    if let Some((ip, prefix)) = phys_ip
        && !has_bridge_ip
    {
        address::remove_ipv4(handle, phys_index, ip).await?;
        address::add_ipv4(handle, br_index, ip, prefix).await?;
        route::restore_default_gateway(handle).await?;

        log!(
            "network",
            "Transferred IP {}/{} to bridge {}",
            ip,
            prefix,
            bridge_name
        );
    }

    Ok(())
}
