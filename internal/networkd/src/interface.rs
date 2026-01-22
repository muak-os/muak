use anyhow::Result;
use futures_util::stream::TryStreamExt;
use netlink_packet_route::link::{LinkAttribute, LinkFlags};
use rtnetlink::Handle;

#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    Up,
    NoCarrier,
    Down,
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub index: u32,
    pub mac_address: [u8; 6],
    pub link_state: LinkState,
}

impl Interface {
    pub fn new(name: String, index: u32, mac_address: [u8; 6], link_state: LinkState) -> Self {
        Self {
            name,
            index,
            mac_address,
            link_state,
        }
    }

    pub fn has_carrier(&self) -> bool {
        self.link_state == LinkState::Up
    }
}

impl std::fmt::Display for LinkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkState::Up => write!(f, "up"),
            LinkState::NoCarrier => write!(f, "no-carrier"),
            LinkState::Down => write!(f, "down"),
        }
    }
}

pub async fn discover_ethernet_interfaces(handle: &Handle) -> Result<Vec<Interface>> {
    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link_msg) = links.try_next().await? {
        let (name, mac_address, is_virtual) = get_link_attributes(&link_msg.attributes);

        if name.is_empty() || !is_ethernet_interface(&name) || is_virtual {
            continue;
        }

        let flags = link_msg.header.flags;
        let is_admin_up = flags.contains(LinkFlags::Up);
        let has_carrier = flags.contains(LinkFlags::LowerUp);

        let link_state = match (is_admin_up, has_carrier) {
            (true, true) => LinkState::Up,
            (true, false) => LinkState::NoCarrier,
            (false, _) => LinkState::Down,
        };

        kmsg::info!(
            "Discovered interface: {} (index {}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, state: {})",
            name,
            link_msg.header.index,
            mac_address[0],
            mac_address[1],
            mac_address[2],
            mac_address[3],
            mac_address[4],
            mac_address[5],
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
                is_virtual = info
                    .iter()
                    .any(|attr| matches!(attr, netlink_packet_route::link::LinkInfo::Kind(_)))
            }
            _ => {}
        }
    }

    (name, mac_address, is_virtual)
}

pub fn is_ethernet_interface(name: &str) -> bool {
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
    pub fn select_primary(interfaces: &[Interface]) -> Option<&Interface> {
        if interfaces.is_empty() {
            return None;
        }

        interfaces
            .iter()
            .max_by(|a, b| Self::compare_interfaces(a, b))
    }

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

        if interface.link_state != LinkState::Down {
            score += 1000;
        }

        score += Self::score_naming(&interface.name);

        score
    }

    fn score_naming(name: &str) -> u32 {
        // Priority order: eno > ens > enp > end > eth
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

    fn make_interface(name: &str, link_state: LinkState) -> Interface {
        make_interface_with_index(name, 0, link_state)
    }

    fn make_interface_with_index(name: &str, index: u32, link_state: LinkState) -> Interface {
        Interface {
            name: name.to_string(),
            index,
            mac_address: [0, 0, 0, 0, 0, 0],
            link_state,
        }
    }

    #[test]
    fn test_is_ethernet_interface() {
        assert!(is_ethernet_interface("eth0"));
        assert!(is_ethernet_interface("enp3s0"));
        assert!(!is_ethernet_interface("lo"));
        assert!(!is_ethernet_interface("wlan0"));
        assert!(!is_ethernet_interface("br0"));
    }

    #[test]
    fn test_select_primary_prefers_carrier() {
        let interfaces = vec![
            make_interface("eth0", LinkState::NoCarrier),
            make_interface("eth1", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth1");
    }

    #[test]
    fn test_select_primary_prefers_no_carrier_over_down() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Down),
            make_interface("eth1", LinkState::NoCarrier),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth1");
    }

    #[test]
    fn test_select_primary_prefers_better_naming() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eno1", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eno1");
    }

    #[test]
    fn test_naming_priority_order() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("enp3s0", LinkState::Up),
            make_interface("ens1", LinkState::Up),
            make_interface("eno1", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eno1");
    }

    #[test]
    fn test_carrier_overrides_naming() {
        let interfaces = vec![
            make_interface("eno1", LinkState::NoCarrier),
            make_interface("eth0", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth0");
    }

    #[test]
    fn test_select_backups_excludes_primary() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Up),
            make_interface("eth2", LinkState::Down),
        ];

        let backups = InterfaceSelector::select_backups(&interfaces, "eth0");
        assert_eq!(backups.len(), 2);
        assert!(backups.iter().all(|i| i.name != "eth0"));
    }

    #[test]
    fn test_select_backups_sorted_by_priority() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Down),
            make_interface("eno1", LinkState::Up),
            make_interface("enp3s0", LinkState::Up),
        ];

        let backups = InterfaceSelector::select_backups(&interfaces, "eth0");
        assert_eq!(backups[0].name, "eno1");
        assert_eq!(backups[1].name, "enp3s0");
        assert_eq!(backups[2].name, "eth1");
    }

    #[test]
    fn test_empty_interfaces() {
        let interfaces: Vec<Interface> = vec![];
        let primary = InterfaceSelector::select_primary(&interfaces);
        assert!(primary.is_none());
    }

    #[test]
    fn test_single_interface() {
        let interfaces = vec![make_interface("eth0", LinkState::Down)];
        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth0");
    }

    #[test]
    fn test_has_carrier() {
        assert!(make_interface("eth0", LinkState::Up).has_carrier());
        assert!(!make_interface("eth0", LinkState::NoCarrier).has_carrier());
        assert!(!make_interface("eth0", LinkState::Down).has_carrier());
    }

    #[test]
    fn test_tiebreaker_prefers_lower_index() {
        let interfaces = vec![
            make_interface_with_index("eth0", 8, LinkState::Down),
            make_interface_with_index("eth1", 9, LinkState::Down),
            make_interface_with_index("eth2", 10, LinkState::Down),
            make_interface_with_index("eth3", 11, LinkState::Down),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth0");
        assert_eq!(primary.unwrap().index, 8);
    }
}
