use anyhow::Result;
use futures::stream::TryStreamExt;
use netlink_packet_route::link::{LinkAttribute, LinkFlags};
use rtnetlink::Handle;

#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    Up,
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
}

impl std::fmt::Display for LinkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkState::Up => write!(f, "up"),
            LinkState::Down => write!(f, "down"),
        }
    }
}

pub async fn discover_ethernet_interfaces(handle: &Handle) -> Result<Vec<Interface>> {
    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link_msg) = links.try_next().await? {
        let mut name = String::new();
        let mut mac_address = [0u8; 6];
        let mut is_virtual = false;

        for attr in &link_msg.attributes {
            match attr {
                LinkAttribute::IfName(n) => name = n.clone(),
                LinkAttribute::Address(addr) if addr.len() == 6 => {
                    mac_address.copy_from_slice(&addr[..6])
                }
                LinkAttribute::LinkInfo(info) => {
                    for link_info_attr in info {
                        if matches!(
                            link_info_attr,
                            netlink_packet_route::link::LinkInfo::Kind(_)
                        ) {
                            is_virtual = true;
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        if name.is_empty() {
            continue;
        }
        if !is_ethernet_interface(&name) {
            continue;
        }
        if is_virtual {
            continue;
        }

        let link_state = if link_msg.header.flags.contains(LinkFlags::Up) {
            LinkState::Up
        } else {
            LinkState::Down
        };

        interfaces.push(Interface::new(
            name,
            link_msg.header.index,
            mac_address,
            link_state,
        ));
    }
    Ok(interfaces)
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

    pub fn select_secondaries<'a>(
        interfaces: &'a [Interface],
        primary_name: &str,
    ) -> Vec<&'a Interface> {
        let mut secondaries: Vec<&Interface> = interfaces
            .iter()
            .filter(|i| i.name != primary_name)
            .collect();

        secondaries.sort_by(|a, b| Self::compare_interfaces(a, b).reverse());

        secondaries
    }

    fn compare_interfaces(a: &Interface, b: &Interface) -> std::cmp::Ordering {
        let score_a = Self::score_interface(a);
        let score_b = Self::score_interface(b);

        score_a.cmp(&score_b)
    }

    fn score_interface(interface: &Interface) -> u32 {
        let mut score = 0u32;

        if interface.link_state == LinkState::Up {
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
        Interface {
            name: name.to_string(),
            index: 0,
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
    fn test_select_primary_prefers_up_interface() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Down),
            make_interface("eth1", LinkState::Up),
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
    fn test_link_state_overrides_naming() {
        let interfaces = vec![
            make_interface("eno1", LinkState::Down),
            make_interface("eth0", LinkState::Up),
        ];

        let primary = InterfaceSelector::select_primary(&interfaces);
        assert_eq!(primary.unwrap().name, "eth0");
    }

    #[test]
    fn test_select_secondaries_excludes_primary() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Up),
            make_interface("eth2", LinkState::Down),
        ];

        let secondaries = InterfaceSelector::select_secondaries(&interfaces, "eth0");
        assert_eq!(secondaries.len(), 2);
        assert!(secondaries.iter().all(|i| i.name != "eth0"));
    }

    #[test]
    fn test_select_secondaries_sorted_by_priority() {
        let interfaces = vec![
            make_interface("eth0", LinkState::Up),
            make_interface("eth1", LinkState::Down),
            make_interface("eno1", LinkState::Up),
            make_interface("enp3s0", LinkState::Up),
        ];

        let secondaries = InterfaceSelector::select_secondaries(&interfaces, "eth0");
        assert_eq!(secondaries[0].name, "eno1");
        assert_eq!(secondaries[1].name, "enp3s0");
        assert_eq!(secondaries[2].name, "eth1");
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
}
