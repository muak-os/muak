//! Ethernet interface enumeration and priority-based selection.

use rtnetlink::Handle;
use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags, LinkInfo};
use thiserror::Error;
use tokio_stream::StreamExt;

use crate::link::LinkStateKind;
use crate::mac::format;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to enumerate links: {0}")]
    List(#[source] rtnetlink::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub index: u32,
    pub mac_address: [u8; 6],
    pub link_state: LinkStateKind,
}

impl Interface {
    /// Constructs a new interface descriptor.
    pub fn new(name: String, index: u32, mac_address: [u8; 6], link_state: LinkStateKind) -> Self {
        Self {
            name,
            index,
            mac_address,
            link_state,
        }
    }

    /// Returns true when the underlying link has an active carrier signal.
    pub fn has_carrier(&self) -> bool {
        self.link_state.has_carrier()
    }
}

/// Discovers all physical ethernet interfaces via rtnetlink.
pub async fn discover_ethernet(handle: &Handle) -> Result<Vec<Interface>> {
    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link_msg) = links.try_next().await.map_err(Error::List)? {
        let (name, mac_address, is_virtual) = get_link_attributes(&link_msg.attributes);

        if name.is_empty() || !is_ethernet(&name) || is_virtual {
            continue;
        }

        let flags = link_msg.header.flags;
        let is_admin_up = flags.contains(LinkFlags::Up);
        let has_carrier = flags.contains(LinkFlags::LowerUp);

        let link_state = match (is_admin_up, has_carrier) {
            (true, true) => LinkStateKind::Up,
            (true, false) => LinkStateKind::NoCarrier,
            (false, _) => LinkStateKind::Down,
        };

        println!(
            "Discovered interface: {} (index {}, MAC {}, state: {})",
            name,
            link_msg.header.index,
            format(&mac_address),
            link_state
        );

        interfaces.push(Interface::new(
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
    let mut mac_address = [0u8; 6];
    let mut is_virtual = false;

    for attr in attributes {
        match attr {
            LinkAttribute::IfName(n) => name = n.clone(),
            LinkAttribute::Address(addr) if addr.len() == 6 => {
                mac_address.copy_from_slice(&addr[..6]);
            }
            LinkAttribute::LinkInfo(info) => {
                is_virtual = info.iter().any(|attr| matches!(attr, LinkInfo::Kind(_)))
            }
            _ => {}
        }
    }

    (name, mac_address, is_virtual)
}

/// Returns true if the name matches a physical ethernet naming convention.
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

pub struct InterfaceSelector;

impl InterfaceSelector {
    /// Selects the highest-priority interface as the primary.
    pub fn select_primary(interfaces: &[Interface]) -> Option<&Interface> {
        if interfaces.is_empty() {
            return None;
        }

        interfaces
            .iter()
            .max_by(|a, b| Self::compare_interfaces(a, b))
    }

    /// Returns all non-primary interfaces sorted by descending priority.
    pub fn select_backups<'a>(
        interfaces: &'a [Interface],
        primary_name: &str,
    ) -> Vec<&'a Interface> {
        let mut backups: Vec<&Interface> = interfaces
            .iter()
            .filter(|i| i.name != primary_name)
            .collect();

        backups.sort_by(|a, b| Self::compare_interfaces(a, b).reverse());

        backups
    }

    fn compare_interfaces(a: &Interface, b: &Interface) -> std::cmp::Ordering {
        let score_a = Self::score_interface(a);
        let score_b = Self::score_interface(b);

        score_a.cmp(&score_b).then_with(|| b.index.cmp(&a.index))
    }

    fn score_interface(interface: &Interface) -> u32 {
        let mut score = 0u32;

        if interface.has_carrier() {
            score += 2000;
        }

        if interface.link_state != LinkStateKind::Down {
            score += 1000;
        }

        score += Self::score_naming(&interface.name);

        score
    }

    fn score_naming(name: &str) -> u32 {
        if name.starts_with("eno") {
            500
        } else if name.starts_with("ens") {
            400
        } else if name.starts_with("enp") {
            300
        } else if name.starts_with("end") {
            200
        } else if name.starts_with("eth") {
            100
        } else {
            50
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_interface(name: &str, link_state: LinkStateKind) -> Interface {
        make_interface_with_index(name, 0, link_state)
    }

    fn make_interface_with_index(name: &str, index: u32, link_state: LinkStateKind) -> Interface {
        Interface {
            name: name.to_string(),
            index,
            mac_address: [0, 0, 0, 0, 0, 0],
            link_state,
        }
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

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, "eth0");

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

        // ACT
        let backups = InterfaceSelector::select_backups(&interfaces, "eth0");

        // ASSERT
        assert_eq!(backups[0].name, "eno1");
        assert_eq!(backups[1].name, "enp3s0");
        assert_eq!(backups[2].name, "eth1");
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
        let p = primary.expect("should have primary");
        assert_eq!(p.name, "eth0");
        assert_eq!(p.index, 8);
    }
}
