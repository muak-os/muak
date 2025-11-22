use anyhow::{Context, Result};
use futures::stream::TryStreamExt;
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use rtnetlink::Handle;
use rtnetlink::LinkUnspec;

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

pub fn extract_name_from_link(link: &LinkMessage) -> Option<String> {
    for attr in &link.attributes {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name.clone());
        }
    }
    None
}
