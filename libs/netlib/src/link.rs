//! Network link operations and link state types.

use std::collections::HashMap;
use std::time::Duration;

use rtnetlink::Handle;
use rtnetlink::LinkUnspec;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use thiserror::Error;
use tokio_stream::StreamExt;

#[derive(Debug, Error)]
pub enum Error {
    #[error("link '{0}' not found")]
    NotFound(String),
    #[error("failed to query link: {0}")]
    Query(#[source] rtnetlink::Error),
    #[error("failed to bring link up: {0}")]
    BringUp(#[source] rtnetlink::Error),
    #[error("failed to bring link down: {0}")]
    BringDown(#[source] rtnetlink::Error),
    #[error("failed to set link master: {0}")]
    SetMaster(#[source] rtnetlink::Error),
    #[error("failed to delete link: {0}")]
    Delete(#[source] rtnetlink::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Finds a link by name, returning its full message or an error if not found.
pub async fn find_by_name(handle: &Handle, name: &str) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    links
        .try_next()
        .await
        .map_err(Error::Query)?
        .ok_or_else(|| Error::NotFound(name.to_string()))
}

/// Returns the kernel interface index for the named link.
pub async fn get_index(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;
    Ok(link.header.index)
}

/// Checks whether a link with the given name exists.
pub async fn exists(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();

    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) | Err(_) => Ok(false),
    }
}

/// Sets the IFF_UP flag on the link identified by index.
pub async fn bring_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .map_err(Error::BringUp)
}

/// Clears the IFF_UP flag on the link identified by index.
pub async fn bring_down(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).down().build())
        .execute()
        .await
        .map_err(Error::BringDown)
}

/// Ensures a named link is administratively up, returning its index.
pub async fn ensure_up(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;
    let index = link.header.index;

    if !link.header.flags.contains(LinkFlags::Up) {
        println!("Bringing up interface {} (index {})", name, index);
        bring_up(handle, index).await?;
    }

    Ok(index)
}

/// Extracts the 6-byte hardware address from a link message, if present.
pub fn extract_mac(link: &LinkMessage) -> Option<[u8; 6]> {
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

/// Sets the master (bridge/bond) for a slave link.
pub async fn set_master(handle: &Handle, slave_index: u32, master_index: u32) -> Result<()> {
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(slave_index)
                .controller(master_index)
                .build(),
        )
        .execute()
        .await
        .map_err(Error::SetMaster)
}

/// Deletes a network link by its index.
pub async fn delete(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .del(index)
        .execute()
        .await
        .map_err(Error::Delete)
}

/// Returns carrier state (LowerUp flag) for every link on the system.
pub async fn get_all_carrier_states(handle: &Handle) -> Result<HashMap<u32, bool>> {
    let mut states = HashMap::new();
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await.map_err(Error::Query)? {
        let has_carrier = link.header.flags.contains(LinkFlags::LowerUp);
        states.insert(link.header.index, has_carrier);
    }

    Ok(states)
}

/// Brings up all listed interfaces and polls until at least one has carrier or timeout.
pub async fn probe_interfaces_for_carrier(
    handle: &Handle,
    interfaces: &[(u32, String)],
    timeout: Duration,
) -> HashMap<u32, bool> {
    let indices: Vec<u32> = interfaces.iter().map(|(idx, _)| *idx).collect();
    let names: Vec<&str> = interfaces.iter().map(|(_, name)| name.as_str()).collect();

    println!(
        "Probing {} interfaces for carrier (timeout: {:?}): {:?}",
        interfaces.len(),
        timeout,
        names
    );

    for &index in &indices {
        let _ = bring_up(handle, index).await;
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
            log_carrier_detections(interfaces, &states, start.elapsed());
            return indices
                .iter()
                .map(|idx| (*idx, states.get(idx) == Some(&true)))
                .collect();
        }

        if start.elapsed() >= timeout {
            println!("No carrier detected on any interface after {:?}", timeout);
            return indices.iter().map(|idx| (*idx, false)).collect();
        }

        tokio::time::sleep(poll_interval).await;
    }
}

fn log_carrier_detections(
    interfaces: &[(u32, String)],
    states: &HashMap<u32, bool>,
    elapsed: Duration,
) {
    for (idx, name) in interfaces {
        if states.get(idx) == Some(&true) {
            println!("Carrier detected on {} after {:?}", name, elapsed);
        }
    }
}

/// Extracts the interface name from a link message, if present.
pub fn extract_name(link: &LinkMessage) -> Option<String> {
    for attr in &link.attributes {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name.clone());
        }
    }
    None
}

/// Administrative and carrier state of a network link.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkStateKind {
    Up,
    NoCarrier,
    Down,
}

impl LinkStateKind {
    /// Returns true when the link has an active carrier signal.
    pub fn has_carrier(&self) -> bool {
        *self == LinkStateKind::Up
    }
}

impl std::fmt::Display for LinkStateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkStateKind::Up => write!(f, "up"),
            LinkStateKind::NoCarrier => write!(f, "no-carrier"),
            LinkStateKind::Down => write!(f, "down"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_state_kind_has_carrier_up() {
        // ACT / ASSERT
        assert!(LinkStateKind::Up.has_carrier());
    }

    #[test]
    fn link_state_kind_has_carrier_no_carrier() {
        // ACT / ASSERT
        assert!(!LinkStateKind::NoCarrier.has_carrier());
    }

    #[test]
    fn link_state_kind_has_carrier_down() {
        // ACT / ASSERT
        assert!(!LinkStateKind::Down.has_carrier());
    }

    #[test]
    fn link_state_kind_display_up() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Up.to_string(), "up");
    }

    #[test]
    fn link_state_kind_display_no_carrier() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::NoCarrier.to_string(), "no-carrier");
    }

    #[test]
    fn link_state_kind_display_down() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Down.to_string(), "down");
    }

    #[test]
    fn link_state_kind_equality() {
        // ACT / ASSERT
        assert_eq!(LinkStateKind::Up, LinkStateKind::Up);
        assert_ne!(LinkStateKind::Up, LinkStateKind::Down);
        assert_ne!(LinkStateKind::NoCarrier, LinkStateKind::Down);
    }
}
