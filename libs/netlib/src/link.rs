//! Network link operations and link state types.

use alloc::string::String;
use core::fmt;
use core::future::Future;
use core::time::Duration;
use std::collections::HashMap;

use rtnetlink::Handle;
use rtnetlink::LinkUnspec;
use rtnetlink::MulticastGroup;
use rtnetlink::packet_core::NetlinkPayload;
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use thiserror::Error;
use tokio::time::{Instant, timeout as timeout_after};
use tokio_stream::StreamExt as _;

use crate::netlink::Rtnl;

/// Link operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Link not found.
    #[error("link '{0}' not found")]
    NotFound(String),
    /// Failed to query link.
    #[error("failed to query link: {0}")]
    Query(#[source] rtnetlink::Error),
    /// Failed to bring link up.
    #[error("failed to bring link up: {0}")]
    BringUp(#[source] rtnetlink::Error),
    /// Failed to bring link down.
    #[error("failed to bring link down: {0}")]
    BringDown(#[source] rtnetlink::Error),
    /// Failed to set link master.
    #[error("failed to set link master: {0}")]
    SetMaster(#[source] rtnetlink::Error),
    /// Failed to delete link.
    #[error("failed to delete link: {0}")]
    Delete(#[source] rtnetlink::Error),
}

/// Link operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Administrative and carrier state of a network link.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Link is administratively up with carrier detected.
    Up,
    /// Link is administratively up but no carrier detected.
    NoCarrier,
    /// Link is administratively down.
    Down,
}

impl State {
    /// Returns true when the link has an active carrier signal.
    #[must_use]
    pub fn has_carrier(&self) -> bool {
        *self == Self::Up
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Up => write!(f, "up"),
            Self::NoCarrier => write!(f, "no-carrier"),
            Self::Down => write!(f, "down"),
        }
    }
}

/// Trait covering all link-layer netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Returns whether a link with the given name exists.
    fn exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Returns the kernel interface index for a named link.
    fn index(&self, name: &str) -> impl Future<Output = Result<u32>> + Send;

    /// Brings a named link up, returning its index.
    fn ensure_up(&self, name: &str) -> impl Future<Output = Result<u32>> + Send;

    /// Brings a link up by its index.
    fn bring_up(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Brings a link down by its index.
    fn bring_down(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Attaches a slave interface to a master (bridge) interface.
    fn set_master(
        &self,
        slave_index: u32,
        master_index: u32,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Deletes a link by its index.
    fn delete(&self, index: u32) -> impl Future<Output = Result<()>> + Send;

    /// Polls interfaces for carrier, returning a map of index to carrier-present.
    fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        timeout: Duration,
    ) -> impl Future<Output = HashMap<u32, bool>> + Send;
}

impl Ops for Rtnl {
    async fn exists(&self, name: &str) -> Result<bool> {
        exists_by_name(&self.handle, name).await
    }

    async fn index(&self, name: &str) -> Result<u32> {
        Ok(find_by_name(&self.handle, name).await?.header.index)
    }

    async fn ensure_up(&self, name: &str) -> Result<u32> {
        ensure_up_by_name(&self.handle, name).await
    }

    async fn bring_up(&self, index: u32) -> Result<()> {
        bring_up(&self.handle, index).await
    }

    async fn bring_down(&self, index: u32) -> Result<()> {
        bring_down(&self.handle, index).await
    }

    async fn set_master(&self, slave_index: u32, master_index: u32) -> Result<()> {
        set_master(&self.handle, slave_index, master_index).await
    }

    async fn delete(&self, index: u32) -> Result<()> {
        delete(&self.handle, index).await
    }

    async fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        timeout: Duration,
    ) -> HashMap<u32, bool> {
        probe_carriers_with_handle(&self.handle, interfaces, timeout).await
    }
}

/// Finds a link by name, returning its full message or an error if not found.
pub(crate) async fn find_by_name(handle: &Handle, name: &str) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();

    links
        .try_next()
        .await
        .map_err(Failure::Query)?
        .ok_or_else(|| Failure::NotFound(name.to_owned()))
}

/// Returns the kernel interface index for the named link.
pub(crate) async fn get_index(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;
    Ok(link.header.index)
}

/// Checks whether a link with the given name exists.
pub(crate) async fn exists(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();

    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(rtnetlink::Error::NetlinkError(ref msg))
            if msg.code.is_some_and(|code| matches!(code.get(), -19 | -2)) =>
        {
            Ok(false)
        }
        Err(error) => Err(Failure::Query(error)),
    }
}

/// Sets the `IFF_UP` flag on the link identified by index.
pub(crate) async fn bring_up(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .map_err(Failure::BringUp)
}

/// Clears the `IFF_UP` flag on the link identified by index.
pub(crate) async fn bring_down(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .set(LinkUnspec::new_with_index(index).down().build())
        .execute()
        .await
        .map_err(Failure::BringDown)
}

/// Sets the master (bridge/bond) for a slave link.
pub(crate) async fn set_master(handle: &Handle, slave_index: u32, master_index: u32) -> Result<()> {
    handle
        .link()
        .set(
            LinkUnspec::new_with_index(slave_index)
                .controller(master_index)
                .build(),
        )
        .execute()
        .await
        .map_err(Failure::SetMaster)
}

/// Deletes a network link by its index.
pub(crate) async fn delete(handle: &Handle, index: u32) -> Result<()> {
    handle
        .link()
        .del(index)
        .execute()
        .await
        .map_err(Failure::Delete)
}

/// Returns the master (bridge/bond) interface index for a link, if any.
pub(crate) async fn master_index(handle: &Handle, index: u32) -> Result<Option<u32>> {
    let mut links = handle.link().get().match_index(index).execute();
    let link = links
        .try_next()
        .await
        .map_err(Failure::Query)?
        .ok_or_else(|| Failure::NotFound(format!("index {index}")))?;
    for attr in &link.attributes {
        if let &LinkAttribute::Controller(master) = attr {
            return Ok(Some(master));
        }
    }

    Ok(None)
}

/// Extracts the 6-byte hardware address from a link message, if present.
pub(crate) fn extract_mac(link: &LinkMessage) -> Option<[u8; 6]> {
    for attr in link.attributes.iter().cloned() {
        if let LinkAttribute::Address(address) = attr
            && let Ok(mac) = <[u8; 6]>::try_from(address.as_slice())
        {
            return Some(mac);
        }
    }
    None
}

/// Extracts the interface name from a link message, if present.
pub(crate) fn extract_name(link: &LinkMessage) -> Option<String> {
    for attr in link.attributes.iter().cloned() {
        if let LinkAttribute::IfName(name) = attr {
            return Some(name.clone());
        }
    }
    None
}

async fn exists_by_name(handle: &Handle, name: &str) -> Result<bool> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();
    match links.try_next().await {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(rtnetlink::Error::NetlinkError(ref msg))
            if msg.code.is_some_and(|code| matches!(code.get(), -19 | -2)) =>
        {
            Ok(false)
        }
        Err(error) => Err(Failure::Query(error)),
    }
}

async fn ensure_up_by_name(handle: &Handle, name: &str) -> Result<u32> {
    let link = find_by_name(handle, name).await?;
    let index = link.header.index;
    if !link.header.flags.contains(LinkFlags::Up) {
        println!("Bringing up interface {name} (index {index})");
        bring_up(handle, index).await?;
    }
    Ok(index)
}

async fn get_all_carrier_states(handle: &Handle) -> Result<HashMap<u32, bool>> {
    let mut states = HashMap::new();
    let mut links = handle.link().get().execute();
    while let Some(link) = links.try_next().await.map_err(Failure::Query)? {
        states.insert(
            link.header.index,
            link.header.flags.contains(LinkFlags::LowerUp),
        );
    }
    Ok(states)
}

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
    interfaces: &[(u32, &str)],
    states: &HashMap<u32, bool>,
    elapsed: Duration,
) {
    for &(index, name) in interfaces {
        if states.get(&index) == Some(&true) {
            println!("Carrier detected on {name} after {elapsed:?}");
        }
    }
}

async fn probe_carriers_with_handle(
    handle: &Handle,
    interfaces: &[(u32, &str)],
    timeout: Duration,
) -> HashMap<u32, bool> {
    let indices: Vec<u32> = interfaces.iter().map(|&(index, _)| index).collect();
    let names: Vec<&str> = interfaces.iter().map(|&(_, name)| name).collect();

    println!(
        "Probing {} interfaces for carrier (timeout: {:?}): {:?}",
        interfaces.len(),
        timeout,
        names
    );

    let (conn, sub_handle, mut messages) =
        match rtnetlink::new_multicast_connection(&[MulticastGroup::Link]) {
            Ok(connection_parts) => connection_parts,
            Err(error) => {
                println!("Failed to open netlink subscription: {error}");
                return indices.iter().map(|idx| (*idx, false)).collect();
            }
        };
    tokio::spawn(conn);

    for &index in &indices {
        let _result = bring_up(handle, index).await;
    }

    let mut states: HashMap<u32, bool> = indices.iter().map(|&idx| (idx, false)).collect();

    if let Ok(initial) = get_all_carrier_states(&sub_handle).await {
        seed_carrier_states(&mut states, &indices, &initial);
    }

    if states.values().any(|&carrier_present| carrier_present) {
        log_carrier_detections(interfaces, &states, Duration::ZERO);
        return states;
    }

    let Some(deadline) = Instant::now().checked_add(timeout) else {
        return states;
    };

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        let next = timeout_after(remaining, messages.next());
        let Ok(Some((message, _))) = next.await else {
            break;
        };

        let NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link_msg)) = message.payload
        else {
            continue;
        };

        let idx = link_msg.header.index;
        if !states.contains_key(&idx) || !link_msg.header.flags.contains(LinkFlags::LowerUp) {
            continue;
        }

        states.insert(idx, true);
        log_carrier_detections(interfaces, &states, timeout.saturating_sub(remaining));
        if states.values().all(|&carrier_present| carrier_present) {
            return states;
        }
    }

    if !states.values().any(|&carrier_present| carrier_present) {
        println!("No carrier detected on any interface after {timeout:?}");
    }

    states
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::collections::HashMap;

    use rtnetlink::packet_route::link::{LinkAttribute, LinkMessage};

    use super::{
        State as LinkStateKind, extract_mac, extract_name, log_carrier_detections,
        seed_carrier_states,
    };

    fn link_with_attributes(attributes: Vec<LinkAttribute>) -> LinkMessage {
        let mut link = LinkMessage::default();
        link.attributes = attributes;
        link
    }

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

    #[test]
    fn extract_mac_returns_six_byte_address() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6])]);

        // ACT
        let mac = extract_mac(&link);

        // ASSERT
        assert_eq!(mac, Some([1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn extract_mac_ignores_invalid_address_length() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3])]);

        // ACT
        let mac = extract_mac(&link);

        // ASSERT
        assert!(mac.is_none());
    }

    #[test]
    fn extract_name_returns_interface_name() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::IfName("eth0".to_owned())]);

        // ACT
        let name = extract_name(&link);

        // ASSERT
        assert_eq!(name.as_deref(), Some("eth0"));
    }

    #[test]
    fn extract_name_returns_none_when_missing() {
        // ARRANGE
        let link = link_with_attributes(vec![LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6])]);

        // ACT
        let name = extract_name(&link);

        // ASSERT
        assert!(name.is_none());
    }

    #[test]
    fn seed_carrier_states_marks_only_requested_carriers() {
        // ARRANGE
        let mut states = HashMap::new();
        let initial = HashMap::from([(1, true), (2, false), (3, true)]);

        // ACT
        seed_carrier_states(&mut states, &[1, 2], &initial);

        // ASSERT
        assert_eq!(states, HashMap::from([(1, true)]));
    }

    #[test]
    fn log_carrier_detections_accepts_absent_states() {
        // ARRANGE
        let interfaces = [(1, "eth0"), (2, "eth1")];
        let states = HashMap::from([(1, true)]);

        // ACT
        log_carrier_detections(&interfaces, &states, Duration::ZERO);

        // ASSERT
        assert_eq!(states.get(&1), Some(&true));
    }
}
