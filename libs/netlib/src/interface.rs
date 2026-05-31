//! Ethernet interface enumeration and priority-based selection.

use alloc::string::String;
use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::future::Future;
use core::str::FromStr;

use rtnetlink::Handle;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkInfo};
use thiserror::Error;
use tokio_stream::StreamExt as _;

use crate::link::State;
use crate::mac::format;
use crate::netlink::Rtnl;

/// A validated Linux network interface name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    const MAX_LEN: usize = 15;

    /// Creates a new [`Name`], validating length and content.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] when the provided interface name is empty, too long, or contains
    /// a NUL byte.
    pub fn new<Source>(name: Source) -> core::result::Result<Self, InvalidName>
    where
        Source: Into<String>,
    {
        let name = name.into();
        if name.is_empty() || name.len() > Self::MAX_LEN || name.contains('\0') {
            return Err(InvalidName(name));
        }
        Ok(Self(name))
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl FromStr for Name {
    type Err = InvalidName;

    fn from_str(source: &str) -> core::result::Result<Self, Self::Err> {
        Self::new(source.to_owned())
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[derive(Debug, Error)]
#[error("invalid interface name: {0:?}")]
pub struct InvalidName(String);

#[derive(Debug, Error)]
pub enum Failure {
    #[error("failed to enumerate links: {0}")]
    List(#[source] rtnetlink::Error),
    #[error("kernel returned invalid interface name: {0}")]
    InvalidName(#[from] InvalidName),
}

pub type Result<T> = core::result::Result<T, Failure>;

#[derive(Debug, Clone)]
pub struct Ethernet {
    pub name: Name,
    pub index: u32,
    pub mac_address: [u8; 6],
    pub link_state: State,
}

impl Ethernet {
    /// Constructs a new interface descriptor.
    #[must_use]
    pub fn new(name: Name, index: u32, mac_address: [u8; 6], link_state: State) -> Self {
        Self {
            name,
            index,
            mac_address,
            link_state,
        }
    }

    /// Returns true when the underlying link has an active carrier signal.
    #[must_use]
    pub fn has_carrier(&self) -> bool {
        self.link_state.has_carrier()
    }
}

/// Trait covering interface enumeration netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Lists all Ethernet interfaces on the system.
    fn discover_ethernet(&self) -> impl Future<Output = Result<Vec<Ethernet>>> + Send;
}

impl Ops for Rtnl {
    async fn discover_ethernet(&self) -> Result<Vec<Ethernet>> {
        discover_ethernet(&self.handle).await
    }
}

async fn discover_ethernet(handle: &Handle) -> Result<Vec<Ethernet>> {
    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link_msg) = links.try_next().await.map_err(Failure::List)? {
        let (raw_name, mac_address, is_virtual) = get_link_attributes(&link_msg.attributes);

        if raw_name.is_empty() || !is_ethernet(&raw_name) || is_virtual {
            continue;
        }

        let name = Name::new(raw_name)?;
        let flags = link_msg.header.flags;
        let is_admin_up = flags.contains(LinkFlags::Up);
        let has_carrier = flags.contains(LinkFlags::LowerUp);

        let link_state = match (is_admin_up, has_carrier) {
            (true, true) => State::Up,
            (true, false) => State::NoCarrier,
            (false, _) => State::Down,
        };

        println!(
            "Discovered interface: {} (index {}, MAC {}, state: {})",
            name,
            link_msg.header.index,
            format(&mac_address),
            link_state
        );

        interfaces.push(Ethernet::new(
            name,
            link_msg.header.index,
            mac_address,
            link_state,
        ));
    }
    Ok(interfaces)
}

fn get_link_attributes(attributes: &[LinkAttribute]) -> (String, [u8; 6], bool) {
    let mut name = String::new();
    let mut mac_address = [0_u8; 6];
    let mut is_virtual = false;

    for attr in attributes.iter().cloned() {
        if let LinkAttribute::IfName(if_name) = attr {
            name = if_name;
            continue;
        }

        if let LinkAttribute::Address(address) = attr {
            mac_address = <[u8; 6]>::try_from(address.as_slice()).unwrap_or(mac_address);
            continue;
        }

        if let LinkAttribute::LinkInfo(info) = attr {
            is_virtual = info
                .iter()
                .any(|link_info| matches!(link_info, LinkInfo::Kind(_)));
        }
    }

    (name, mac_address, is_virtual)
}

/// Returns true if the name matches a physical Ethernet naming convention.
#[must_use]
pub fn is_ethernet(name: &str) -> bool {
    if name == "lo" || name.starts_with("wlan") || name.starts_with("wlp") {
        return false;
    }
    name.starts_with("eth")
        || name.starts_with("enp")
        || name.starts_with("ens")
        || name.starts_with("eno")
        || name.starts_with("end")
}

pub struct Selector;

impl Selector {
    /// Selects the highest-priority interface as the primary.
    #[must_use]
    pub fn select_primary(interfaces: &[Ethernet]) -> Option<&Ethernet> {
        if interfaces.is_empty() {
            return None;
        }

        interfaces
            .iter()
            .max_by(|left, right| Self::compare_interfaces(left, right))
    }

    /// Returns all non-primary interfaces sorted by descending priority.
    #[must_use]
    pub fn select_backups<'a>(
        interfaces: &'a [Ethernet],
        primary_name: &Name,
    ) -> Vec<&'a Ethernet> {
        let mut backups: Vec<&Ethernet> = interfaces
            .iter()
            .filter(|interface| &interface.name != primary_name)
            .collect();

        backups.sort_by(|left, right| Self::compare_interfaces(left, right).reverse());

        backups
    }

    fn compare_interfaces(left: &Ethernet, right: &Ethernet) -> Ordering {
        let score_left = Self::score_interface(left);
        let score_right = Self::score_interface(right);

        score_left
            .cmp(&score_right)
            .then_with(|| right.index.cmp(&left.index))
    }

    fn score_interface(interface: &Ethernet) -> u32 {
        let mut score = 0_u32;

        if interface.has_carrier() {
            score = score.saturating_add(2000);
        }

        if interface.link_state != State::Down {
            score = score.saturating_add(1000);
        }

        score = score.saturating_add(Self::score_naming(&interface.name));

        score
    }

    fn score_naming(name: &Name) -> u32 {
        let interface_name = name.as_str();
        if interface_name.starts_with("eno") {
            500
        } else if interface_name.starts_with("ens") {
            400
        } else if interface_name.starts_with("enp") {
            300
        } else if interface_name.starts_with("end") {
            200
        } else if interface_name.starts_with("eth") {
            100
        } else {
            50
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::State as LinkStateKind;

    type Interface = Ethernet;
    type InterfaceSelector = Selector;

    use rtnetlink::packet_route::link::InfoKind;

    fn make_interface(name: &str, link_state: LinkStateKind) -> Interface {
        make_interface_with_index(name, 0, link_state)
    }

    fn make_interface_with_index(name: &str, index: u32, link_state: LinkStateKind) -> Interface {
        Interface {
            name: Name::new(name).unwrap(),
            index,
            mac_address: [0, 0, 0, 0, 0, 0],
            link_state,
        }
    }

    #[test]
    fn interface_name_rejects_empty() {
        // ACT / ASSERT
        Name::new("").unwrap_err();
    }

    #[test]
    fn interface_name_rejects_too_long() {
        // ACT / ASSERT
        Name::new("a".repeat(16)).unwrap_err();
    }

    #[test]
    fn interface_name_rejects_null_byte() {
        // ACT / ASSERT
        Name::new("eth\0").unwrap_err();
    }

    #[test]
    fn interface_name_accepts_valid() {
        // ACT / ASSERT
        Name::new("eth0").unwrap();
        Name::new("enp3s0f0").unwrap();
        Name::new("a".repeat(15)).unwrap();
    }

    #[test]
    fn interface_name_partial_eq_str() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();

        // ACT / ASSERT
        assert_eq!(name, "eth0");
        assert_ne!(name, "eth1");
    }

    #[test]
    fn interface_name_display_as_ref_borrow_and_from_str() {
        // ARRANGE
        let name: Name = "eth0".parse().expect("valid name");

        // ACT
        let display = name.to_string();
        let as_ref = name.as_ref();
        let borrowed = core::borrow::Borrow::<str>::borrow(&name);

        // ASSERT
        assert_eq!(display, "eth0");
        assert_eq!(as_ref, "eth0");
        assert_eq!(borrowed, "eth0");
    }

    #[test]
    fn ethernet_new_populates_fields() {
        // ARRANGE
        let name = Name::new("eth0").expect("valid name");
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        let interface = Interface::new(name.clone(), 7, mac, LinkStateKind::Up);

        // ASSERT
        assert_eq!(interface.name, name);
        assert_eq!(interface.index, 7);
        assert_eq!(interface.mac_address, mac);
        assert_eq!(interface.link_state, LinkStateKind::Up);
    }

    #[test]
    fn get_link_attributes_extracts_name_mac_and_virtual_flag() {
        // ARRANGE
        let attributes = vec![
            LinkAttribute::IfName("eth0".to_owned()),
            LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6]),
            LinkAttribute::LinkInfo(vec![LinkInfo::Kind(InfoKind::Veth)]),
        ];

        // ACT
        let (name, mac, is_virtual) = get_link_attributes(&attributes);

        // ASSERT
        assert_eq!(name, "eth0");
        assert_eq!(mac, [1, 2, 3, 4, 5, 6]);
        assert!(is_virtual);
    }

    #[test]
    fn get_link_attributes_ignores_invalid_mac_length() {
        // ARRANGE
        let attributes = vec![
            LinkAttribute::IfName("eth0".to_owned()),
            LinkAttribute::Address(vec![1, 2, 3]),
        ];

        // ACT
        let (name, mac, is_virtual) = get_link_attributes(&attributes);

        // ASSERT
        assert_eq!(name, "eth0");
        assert_eq!(mac, [0; 6]);
        assert!(!is_virtual);
    }

    #[test]
    fn ethernet_interface_name_detection() {
        // ACT / ASSERT
        assert!(is_ethernet("eth0"));
        assert!(is_ethernet("enp3s0"));
        assert!(!is_ethernet("lo"));
        assert!(!is_ethernet("wlan0"));
        assert!(!is_ethernet("br0"));
    }

    #[test]
    fn select_primary_prefers_carrier() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::NoCarrier),
            make_interface("eth1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn select_primary_prefers_no_carrier_over_down() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Down),
            make_interface("eth1", LinkStateKind::NoCarrier),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth1");
    }

    #[test]
    fn select_primary_prefers_better_naming() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eno1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn naming_priority_order() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("enp3s0", LinkStateKind::Up),
            make_interface("ens1", LinkStateKind::Up),
            make_interface("eno1", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eno1");
    }

    #[test]
    fn select_primary_scores_end_above_eth() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("end0", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "end0");
    }

    #[test]
    fn select_primary_handles_unknown_ethernet_like_name() {
        // ARRANGE
        let interfaces = vec![
            make_interface("enx001122334455", LinkStateKind::Up),
            make_interface("eth0", LinkStateKind::NoCarrier),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(
            primary.expect("should have primary").name,
            "enx001122334455"
        );
    }

    #[test]
    fn carrier_overrides_naming() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eno1", LinkStateKind::NoCarrier),
            make_interface("eth0", LinkStateKind::Up),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn select_backups_excludes_primary() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eth1", LinkStateKind::Up),
            make_interface("eth2", LinkStateKind::Down),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, &primary_name);

        // ASSERT
        assert_eq!(backups.len(), 2);
        assert!(backups.iter().all(|i| i.name != "eth0"));
    }

    #[test]
    fn select_backups_sorted_by_priority() {
        // ARRANGE
        let interfaces = vec![
            make_interface("eth0", LinkStateKind::Up),
            make_interface("eth1", LinkStateKind::Down),
            make_interface("eno1", LinkStateKind::Up),
            make_interface("enp3s0", LinkStateKind::Up),
        ];
        let primary_name = Name::new("eth0").unwrap();

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, &primary_name);
        let backup_names: Vec<_> = backups
            .iter()
            .map(|interface| interface.name.as_str())
            .collect();

        // ASSERT
        assert_eq!(backup_names.as_slice(), ["eno1", "enp3s0", "eth1"]);
    }

    #[test]
    fn empty_interfaces() {
        // ARRANGE
        let interfaces: Vec<Interface> = vec![];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert!(primary.is_none());
    }

    #[test]
    fn single_interface() {
        // ARRANGE
        let interfaces = vec![make_interface("eth0", LinkStateKind::Down)];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        assert_eq!(primary.expect("should have primary").name, "eth0");
    }

    #[test]
    fn has_carrier() {
        // ACT & ASSERT
        assert!(make_interface("eth0", LinkStateKind::Up).has_carrier());
        assert!(!make_interface("eth0", LinkStateKind::NoCarrier).has_carrier());
        assert!(!make_interface("eth0", LinkStateKind::Down).has_carrier());
    }

    #[test]
    fn tiebreaker_prefers_lower_index() {
        // ARRANGE
        let interfaces = vec![
            make_interface_with_index("eth0", 8, LinkStateKind::Down),
            make_interface_with_index("eth1", 9, LinkStateKind::Down),
            make_interface_with_index("eth2", 10, LinkStateKind::Down),
            make_interface_with_index("eth3", 11, LinkStateKind::Down),
        ];

        // ACT
        let primary = InterfaceSelector::select_primary(&interfaces);

        // ASSERT
        let primary_interface = primary.expect("should have primary");
        assert_eq!(primary_interface.name, "eth0");
        assert_eq!(primary_interface.index, 8);
    }
}
