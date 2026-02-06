use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::Handle;
use rtnetlink::LinkUnspec;
use tokio_stream::StreamExt;

pub async fn find_link_by_name(handle: &Handle, name: &str) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    links
        .try_next()
        .await
        .context("failed to query link")?
        .ok_or_else(|| anyhow::anyhow!("link '{}' not found", name))
}

pub async fn get_link_index(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_link_by_name(handle, name).await?;
    Ok(link.header.index)
}

pub async fn link_exists(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(_) => Ok(false),
    }
}

pub async fn bring_link_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .context("failed to bring link up")
}

pub async fn bring_link_down(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).down().build())
        .execute()
        .await
        .context("failed to bring link down")
}

pub async fn ensure_link_up(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_link_by_name(handle, name).await?;
    let index = link.header.index;

    if !link.header.flags.contains(LinkFlags::Up) {
        kmsg::info!("Bringing up interface {} (index {})", name, index);
        bring_link_up(handle, index).await?;
    }

    Ok(index)
}

pub fn extract_mac_from_link(link: &LinkMessage) -> Option<[u8; 6]> {
    for attr in &link.attributes {
        if let LinkAttribute::Address(addr) = attr
            && addr.len() == 6
        {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&addr[..6]);
            return Some(mac);
        }
    }
    None
}

pub async fn set_link_master(handle: &Handle, slave_index: u32, master_index: u32) -> Result<()> {
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(slave_index)
                .controller(master_index)
                .build(),
        )
        .execute()
        .await
        .context("failed to set link master")
}

pub async fn delete_link(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .del(index)
        .execute()
        .await
        .context("failed to delete link")
}

pub async fn get_all_carrier_states(handle: &Handle) -> Result<HashMap<u32, bool>> {
    let mut states = HashMap::new();
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        let has_carrier = link.header.flags.contains(LinkFlags::LowerUp);
        states.insert(link.header.index, has_carrier);
    }

    Ok(states)
}

pub async fn probe_interfaces_for_carrier(
    handle: &Handle,
    interfaces: &[(u32, String)],
    timeout: Duration,
) -> HashMap<u32, bool> {
    let indices: Vec<u32> = interfaces.iter().map(|(idx, _)| *idx).collect();
    let names: Vec<&str> = interfaces.iter().map(|(_, name)| name.as_str()).collect();

    kmsg::info!(
        "Probing {} interfaces for carrier (timeout: {:?}): {:?}",
        interfaces.len(),
        timeout,
        names
    );

    for &index in &indices {
        let _ = bring_link_up(handle, index).await;
    }

    let poll_interval = Duration::from_millis(100);
    let start = std::time::Instant::now();

    loop {
        let states = match get_all_carrier_states(handle).await {
            Ok(s) => s,
            Err(_) => {
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        let any_carrier = indices.iter().any(|idx| states.get(idx) == Some(&true));
        if any_carrier {
            for (idx, name) in interfaces {
                if states.get(idx) == Some(&true) {
                    kmsg::info!("Carrier detected on {} after {:?}", name, start.elapsed());
                }
            }
            return indices
                .iter()
                .map(|idx| (*idx, states.get(idx) == Some(&true)))
                .collect();
        }

        if start.elapsed() >= timeout {
            kmsg::warn!("No carrier detected on any interface after {:?}", timeout);
            return indices.iter().map(|idx| (*idx, false)).collect();
        }

        tokio::time::sleep(poll_interval).await;
    }
}

pub fn extract_name_from_link(link: &LinkMessage) -> Option<String> {
    for attr in &link.attributes {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name.clone());
        }
    }
    None
}
