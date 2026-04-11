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
        Ok(None) => Ok(false),
        Err(rtnetlink::Error::NetlinkError(ref msg))
            if msg.code.is_some_and(|c| matches!(c.get(), -19 | -2)) =>
        {
            Ok(false)
        }
        Err(e) => Err(Error::Query(e)),
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

/// Brings up all listed interfaces and waits for carrier via RTM_NEWLINK events.
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

    let (conn, sub_handle, mut messages) = match rtnetlink::new_connection() {
        Ok(t) => t,
        Err(e) => {
            println!("Failed to open netlink subscription: {}", e);
            return indices.iter().map(|idx| (*idx, false)).collect();
        }
    };
    tokio::spawn(conn);

    for &index in &indices {
        let _ = bring_up(handle, index).await;
    }

    let mut states: HashMap<u32, bool> = indices.iter().map(|&idx| (idx, false)).collect();

    if let Ok(initial) = get_all_carrier_states(&sub_handle).await {
        seed_carrier_states(&mut states, &indices, &initial);
    }

    if states.values().any(|&c| c) {
        log_carrier_detections(interfaces, &states, Duration::ZERO);
        return states;
    }

    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let next = tokio::time::timeout(remaining, messages.next());
        let Ok(Some((message, _))) = next.await else {
            break;
        };

        let rtnetlink::packet_core::NetlinkPayload::InnerMessage(
            rtnetlink::packet_route::RouteNetlinkMessage::NewLink(link_msg),
        ) = message.payload
        else {
            continue;
        };

        let idx = link_msg.header.index;
        if !states.contains_key(&idx) || !link_msg.header.flags.contains(LinkFlags::LowerUp) {
            continue;
        }

        states.insert(idx, true);
        log_carrier_detections(interfaces, &states, timeout - remaining);
        if states.values().all(|&c| c) {
            return states;
        }
    }

    if !states.values().any(|&c| c) {
        println!("No carrier detected on any interface after {:?}", timeout);
    }

    states
}

/// Copies initial carrier flags from a full system dump into the tracked states map.
fn seed_carrier_states(
    states: &mut HashMap<u32, bool>,
    indices: &[u32],
    initial: &HashMap<u32, bool>,
) {
    for &idx in indices {
        if initial.get(&idx) == Some(&true) {
            states.insert(idx, true);
        }
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
