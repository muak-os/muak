use crate::log;
use futures::stream::TryStreamExt;
use netlink_packet_route::link::LinkAttribute;
use rtnetlink::Handle;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    Up,
    Down,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub index: u32,
    pub mac_address: [u8; 6],
    pub link_state: LinkState,
    pub ip_config: Option<IpConfig>,
    pub dhcp_lease: Option<DhcpLease>,
    pub last_seen: SystemTime,
}

#[derive(Debug, Clone)]
pub struct IpConfig {
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct DhcpLease {
    pub obtained_at: SystemTime,
    pub lease_time: Duration,
    pub renewal_time: Duration, // T1 - 50% of lease_time
    pub rebind_time: Duration,  // T2 - 87.5% of lease_time
    pub server_id: Ipv4Addr,
}

impl DhcpLease {
    pub fn expiry_time(&self) -> SystemTime {
        self.obtained_at + self.lease_time
    }

    pub fn renewal_deadline(&self) -> SystemTime {
        self.obtained_at + self.renewal_time
    }

    pub fn rebind_deadline(&self) -> SystemTime {
        self.obtained_at + self.rebind_time
    }

    pub fn is_expired(&self) -> bool {
        SystemTime::now() > self.expiry_time()
    }

    pub fn should_renew(&self) -> bool {
        SystemTime::now() > self.renewal_deadline()
    }

    pub fn should_rebind(&self) -> bool {
        SystemTime::now() > self.rebind_deadline()
    }
}

impl Interface {
    pub fn new(name: String, index: u32, mac_address: [u8; 6], link_state: LinkState) -> Self {
        Self {
            name,
            index,
            mac_address,
            link_state,
            ip_config: None,
            dhcp_lease: None,
            last_seen: SystemTime::now(),
        }
    }

    pub fn touch(&mut self) {
        self.last_seen = SystemTime::now();
    }

    pub fn is_operational(&self) -> bool {
        self.link_state == LinkState::Up && self.ip_config.is_some()
    }

    pub fn set_ip_config(&mut self, ip_config: IpConfig) {
        self.ip_config = Some(ip_config);
        self.touch();
    }

    pub fn set_dhcp_lease(&mut self, dhcp_lease: DhcpLease) {
        self.dhcp_lease = Some(dhcp_lease);
        self.touch();
    }

    pub fn set_link_state(&mut self, link_state: LinkState) {
        self.link_state = link_state;
        self.touch();
    }
}

impl std::fmt::Display for LinkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkState::Up => write!(f, "up"),
            LinkState::Down => write!(f, "down"),
            LinkState::Unknown => write!(f, "unknown"),
        }
    }
}

pub async fn setup_loopback(handle: &Handle) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Setting up loopback interface");

    let mut links = handle.link().get().match_name("lo".to_string()).execute();
    if let Some(link) = links.try_next().await? {
        handle.link().set(link.header.index).up().execute().await?;
        log!("network", "Loopback interface is up");
    }

    Ok(())
}

pub async fn discover_ethernet_interfaces(
    handle: &Handle,
) -> Result<Vec<Interface>, Box<dyn std::error::Error>> {
    log!("network", "Discovering ethernet interfaces");

    let mut interfaces = Vec::new();
    let mut links = handle.link().get().execute();

    while let Some(link) = links.try_next().await? {
        let mut name = String::new();
        let mut mac_address = [0u8; 6];
        let mut is_virtual = false;

        for attr in &link.attributes {
            match attr {
                LinkAttribute::IfName(n) => {
                    name = n.clone();
                }
                LinkAttribute::Address(addr) if addr.len() == 6 => {
                    mac_address.copy_from_slice(&addr[..6]);
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
            log!(
                "network",
                "Skipping non-ethernet interface: {} (not matching eth/en patterns)",
                name
            );
            continue;
        }

        if is_virtual {
            log!(
                "network",
                "Skipping virtual interface: {} (has LinkInfo)",
                name
            );
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

        log!(
            "network",
            "Discovered ethernet interface: {} (index: {}, state: {}, mac: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
            name,
            link.header.index,
            link_state,
            mac_address[0],
            mac_address[1],
            mac_address[2],
            mac_address[3],
            mac_address[4],
            mac_address[5]
        );

        interfaces.push(Interface::new(
            name,
            link.header.index,
            mac_address,
            link_state,
        ));
    }

    log!(
        "network",
        "Discovered {} ethernet interface(s)",
        interfaces.len()
    );

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

pub async fn bring_up_interface(
    interface: &str,
    handle: &Handle,
) -> Result<u32, Box<dyn std::error::Error>> {
    log!("network", "Bringing up interface {}", interface);

    let mut links = handle
        .link()
        .get()
        .match_name(interface.to_string())
        .execute();

    let link_index = if let Some(link) = links.try_next().await? {
        let index = link.header.index;
        handle.link().set(index).up().execute().await?;
        log!("network", "Interface {} is up", interface);
        index
    } else {
        return Err("Interface not found".into());
    };

    Ok(link_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ethernet_interface() {
        assert!(is_ethernet_interface("eth0"));
        assert!(is_ethernet_interface("eth1"));
        assert!(is_ethernet_interface("enp3s0"));
        assert!(is_ethernet_interface("ens1"));
        assert!(is_ethernet_interface("eno1"));
        assert!(is_ethernet_interface("end0"));

        assert!(!is_ethernet_interface("lo"));
        assert!(!is_ethernet_interface("wlan0"));
        assert!(!is_ethernet_interface("wlp2s0"));
        assert!(!is_ethernet_interface("docker0"));
        assert!(!is_ethernet_interface("br0"));
    }
}
