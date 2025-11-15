use crate::log;
use anyhow::Result;
use futures::stream::TryStreamExt;
use netlink_packet_route::link::LinkAttribute;
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

pub async fn discover_ethernet_interfaces(
    handle: &Handle,
) -> Result<Vec<Interface>> {
    log!("network", "Discovering ethernet interfaces");
    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        let mut name = String::new();
        let mut mac_address = [0u8; 6];
        let mut is_virtual = false;
        for attr in &link.attributes {
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
        let is_up = link
            .header
            .flags
            .iter()
            .any(|flag| matches!(flag, netlink_packet_route::link::LinkFlag::Up));
        let link_state = if is_up {
            LinkState::Up
        } else {
            LinkState::Down
        };
        interfaces.push(Interface::new(
            name,
            link.header.index,
            mac_address,
            link_state,
        ));
    }
    Ok(interfaces)
}

fn is_ethernet_interface(name: &str) -> bool {
    if name == "lo" || name.starts_with("wlan") || name.starts_with("wlp") {
        return false;
    }
    name.starts_with("eth")
        || name.starts_with("enp")
        || name.starts_with("ens")
        || name.starts_with("eno")
        || name.starts_with("end")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_is_ethernet_interface() {
        assert!(is_ethernet_interface("eth0"));
        assert!(is_ethernet_interface("enp3s0"));
        assert!(!is_ethernet_interface("lo"));
        assert!(!is_ethernet_interface("wlan0"));
        assert!(!is_ethernet_interface("br0"));
    }
}
