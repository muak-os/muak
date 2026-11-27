//! Conversion from the system network configuration to the network model configuration.

use core::net::{Ipv4Addr, Ipv6Addr};

use config::system::network;

struct Dns {
    v4: Vec<Ipv4Addr>,
    v6: Vec<Ipv6Addr>,
}

/// Converts the system network configuration into a network model configuration.
#[must_use = "the caller needs the network configuration to build the loop"]
pub fn convert(config: &network::Config) -> netmodel::Config {
    let dns = Dns {
        v4: config.ipv4_dns().collect(),
        v6: config.ipv6_dns().collect(),
    };

    netmodel::Config {
        ipv6: config.ipv6,
        dns: config.dns.clone(),
        interfaces: config
            .interfaces
            .iter()
            .map(|interface| interface_config(&dns, config.ipv6, interface))
            .collect(),
    }
}

fn interface_config(
    dns: &Dns,
    ipv6_enabled: bool,
    source: &network::InterfaceConfig,
) -> netmodel::InterfaceConfig {
    netmodel::InterfaceConfig {
        name: source.name.clone(),
        kind: kind(&source.kind),
        ipv4: source
            .ipv4
            .as_ref()
            .and_then(|ipv4| ipv4_config(&dns.v4, ipv4)),
        ipv6: source
            .ipv6
            .as_ref()
            .and_then(|ipv6| ipv6_config(ipv6_enabled, &dns.v6, ipv6)),
        bridge: source.bridge.as_ref().map(bridge_config),
    }
}

const fn kind(kind: &network::InterfaceKind) -> netmodel::InterfaceKind {
    match *kind {
        network::InterfaceKind::Bridge => netmodel::InterfaceKind::Bridge,
        network::InterfaceKind::Ethernet => netmodel::InterfaceKind::Ethernet,
    }
}

fn bridge_config(config: &network::BridgeConfig) -> netmodel::BridgeConfig {
    netmodel::BridgeConfig {
        port: config.port.clone(),
        stp: config.stp,
    }
}

#[cfg(feature = "ipv4")]
fn ipv4_config(
    dns: &[Ipv4Addr],
    source: &network::Ipv4InterfaceConfig,
) -> Option<netmodel::Ipv4Config> {
    if source.dhcp {
        return Some(netmodel::Ipv4Config {
            addresses: Vec::new(),
            gateway: None,
            dns: dns.to_vec(),
            dhcp: true,
        });
    }
    if source.addresses.is_empty() {
        return None;
    }

    Some(netmodel::Ipv4Config {
        addresses: source
            .addresses
            .iter()
            .map(|cidr| (cidr.address, cidr.prefix))
            .collect(),
        gateway: source.gateway,
        dns: dns.to_vec(),
        dhcp: false,
    })
}

#[cfg(not(feature = "ipv4"))]
fn ipv4_config(
    _dns: &[Ipv4Addr],
    _source: &network::Ipv4InterfaceConfig,
) -> Option<netmodel::Ipv4Config> {
    None
}

#[cfg(feature = "ipv6")]
fn ipv6_config(
    ipv6_enabled: bool,
    dns: &[Ipv6Addr],
    source: &network::Ipv6InterfaceConfig,
) -> Option<netmodel::Ipv6Config> {
    if !source.addresses.is_empty() {
        return Some(netmodel::Ipv6Config {
            addresses: source
                .addresses
                .iter()
                .map(|cidr| (cidr.address, cidr.prefix))
                .collect(),
            gateway: source.gateway,
            dns: dns.to_vec(),
            autoconf: false,
        });
    }
    if source.autoconf && ipv6_enabled {
        return Some(netmodel::Ipv6Config {
            addresses: Vec::new(),
            gateway: None,
            dns: dns.to_vec(),
            autoconf: true,
        });
    }

    None
}

#[cfg(not(feature = "ipv6"))]
fn ipv6_config(
    _ipv6_enabled: bool,
    _dns: &[Ipv6Addr],
    _source: &network::Ipv6InterfaceConfig,
) -> Option<netmodel::Ipv6Config> {
    None
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ipv6")]
    use core::net::Ipv6Addr;
    use core::net::{IpAddr, Ipv4Addr};

    use super::*;

    fn interface(name: &str, kind: network::InterfaceKind) -> network::InterfaceConfig {
        network::InterfaceConfig {
            name: name.to_owned(),
            kind,
            ipv4: None,
            ipv6: None,
            bridge: None,
        }
    }

    fn mixed_dns() -> Vec<IpAddr> {
        vec![
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V6("2001:4860:4860::8888".parse().expect("valid address")),
        ]
    }

    #[test]
    fn convert_empty_config_has_no_interfaces() {
        // ARRANGE
        let source = network::Config::default();

        // ACT
        let config = convert(&source);

        // ASSERT
        assert!(!config.ipv6);
        assert_eq!(config.dns, Vec::<IpAddr>::new());
        assert_eq!(config.interfaces, Vec::<netmodel::InterfaceConfig>::new());
    }

    #[test]
    fn convert_copies_dns_and_ipv6_toggle() {
        // ARRANGE
        let source = network::Config {
            ipv6: true,
            dns: mixed_dns(),
            interfaces: Vec::new(),
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        assert!(config.ipv6);
        assert_eq!(config.dns, mixed_dns());
    }

    #[test]
    fn convert_maps_ethernet_kind() {
        // ARRANGE
        let source = network::Config {
            ipv6: false,
            dns: Vec::new(),
            interfaces: vec![interface("eth0", network::InterfaceKind::Ethernet)],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        assert_eq!(config.interfaces[0].kind, netmodel::InterfaceKind::Ethernet);
    }

    #[test]
    fn convert_maps_bridge_config() {
        // ARRANGE
        let mut iface = interface("br0", network::InterfaceKind::Bridge);
        iface.bridge = Some(network::BridgeConfig {
            port: vec!["eth0".to_owned(), "eth1".to_owned()],
            stp: true,
        });
        let source = network::Config {
            ipv6: false,
            dns: Vec::new(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        assert_eq!(config.interfaces[0].kind, netmodel::InterfaceKind::Bridge);
        let bridge = config.interfaces[0].bridge.as_ref().expect("bridge config");
        assert_eq!(bridge.port, vec!["eth0".to_owned(), "eth1".to_owned()]);
        assert!(bridge.stp);
    }

    #[cfg(feature = "ipv4")]
    #[test]
    fn convert_ipv4_dhcp_uses_config_dns() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv4 = Some(network::Ipv4InterfaceConfig {
            dhcp: true,
            addresses: Vec::new(),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
        });
        let source = network::Config {
            ipv6: false,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        let ipv4 = config.interfaces[0].ipv4.as_ref().expect("ipv4 config");
        assert!(ipv4.dhcp);
        assert_eq!(ipv4.addresses, Vec::<(Ipv4Addr, u8)>::new());
        assert_eq!(ipv4.gateway, None);
        assert_eq!(ipv4.dns, vec![Ipv4Addr::new(8, 8, 8, 8)]);
    }

    #[cfg(feature = "ipv4")]
    #[test]
    fn convert_ipv4_static_addresses_and_gateway() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv4 = Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        });
        let source = network::Config {
            ipv6: false,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        let ipv4 = config.interfaces[0].ipv4.as_ref().expect("ipv4 config");
        assert!(!ipv4.dhcp);
        assert_eq!(ipv4.addresses, vec![(Ipv4Addr::new(10, 0, 0, 2), 24)]);
        assert_eq!(ipv4.gateway, Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(ipv4.dns, vec![Ipv4Addr::new(8, 8, 8, 8)]);
    }

    #[cfg(feature = "ipv4")]
    #[test]
    fn convert_ipv4_without_dhcp_or_addresses_is_none() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv4 = Some(network::Ipv4InterfaceConfig::default());
        let source = network::Config {
            ipv6: false,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        assert!(config.interfaces[0].ipv4.is_none());
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn convert_ipv6_static_addresses_and_gateway() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv6 = Some(network::Ipv6InterfaceConfig {
            autoconf: true,
            addresses: vec![network::Cidr6 {
                address: "2001:db8::2".parse().expect("valid address"),
                prefix: 64,
            }],
            gateway: Some("2001:db8::1".parse().expect("valid address")),
        });
        let source = network::Config {
            ipv6: true,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        let ipv6 = config.interfaces[0].ipv6.as_ref().expect("ipv6 config");
        assert!(!ipv6.autoconf);
        assert_eq!(
            ipv6.addresses,
            vec![(
                "2001:db8::2".parse::<Ipv6Addr>().expect("valid address"),
                64
            )]
        );
        assert_eq!(
            ipv6.gateway,
            Some("2001:db8::1".parse::<Ipv6Addr>().expect("valid address"))
        );
        assert_eq!(
            ipv6.dns,
            vec![
                "2001:4860:4860::8888"
                    .parse::<Ipv6Addr>()
                    .expect("valid address")
            ]
        );
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn convert_ipv6_autoconf_requires_ipv6_toggle() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv6 = Some(network::Ipv6InterfaceConfig {
            autoconf: true,
            addresses: Vec::new(),
            gateway: None,
        });
        let disabled = network::Config {
            ipv6: false,
            dns: mixed_dns(),
            interfaces: vec![iface.clone()],
        };
        let enabled = network::Config {
            ipv6: true,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let disabled = convert(&disabled);
        let enabled = convert(&enabled);

        // ASSERT
        assert!(disabled.interfaces[0].ipv6.is_none());
        let ipv6 = enabled.interfaces[0].ipv6.as_ref().expect("ipv6 config");
        assert!(ipv6.autoconf);
        assert_eq!(ipv6.addresses, Vec::<(Ipv6Addr, u8)>::new());
        assert_eq!(ipv6.gateway, None);
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn convert_ipv6_without_autoconf_or_addresses_is_none() {
        // ARRANGE
        let mut iface = interface("eth0", network::InterfaceKind::Ethernet);
        iface.ipv6 = Some(network::Ipv6InterfaceConfig::default());
        let source = network::Config {
            ipv6: true,
            dns: mixed_dns(),
            interfaces: vec![iface],
        };

        // ACT
        let config = convert(&source);

        // ASSERT
        assert!(config.interfaces[0].ipv6.is_none());
    }
}
