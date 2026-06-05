use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Network configuration covering DNS, IPv6 toggle, and interfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Whether IPv6 networking is enabled.
    pub ipv6: bool,
    /// DNS server addresses for name resolution.
    pub dns: Vec<IpAddr>,
    /// Declarative list of network interfaces.
    pub interfaces: Vec<InterfaceConfig>,
}

impl NetworkConfig {
    /// Returns an iterator over the IPv4 DNS addresses.
    pub fn ipv4_dns(&self) -> impl Iterator<Item = Ipv4Addr> + '_ {
        self.dns.iter().filter_map(|a| match a {
            IpAddr::V4(v) => Some(*v),
            IpAddr::V6(_) => None,
        })
    }

    /// Returns an iterator over the IPv6 DNS addresses.
    pub fn ipv6_dns(&self) -> impl Iterator<Item = Ipv6Addr> + '_ {
        self.dns.iter().filter_map(|a| match a {
            IpAddr::V6(v) => Some(*v),
            IpAddr::V4(_) => None,
        })
    }
}

/// Declarative configuration for a single network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceConfig {
    /// Interface name (e.g. `eth0`, `br0`).
    pub name: String,
    /// Interface type: `bridge` or `ethernet`.
    #[serde(rename = "type")]
    pub kind: InterfaceKind,
    /// Optional IPv4 configuration.
    #[serde(default)]
    pub ipv4: Option<Ipv4InterfaceConfig>,
    /// Optional IPv6 configuration.
    #[serde(default)]
    pub ipv6: Option<Ipv6InterfaceConfig>,
    /// Optional bridge-specific configuration.
    #[serde(default)]
    pub bridge: Option<BridgeConfig>,
}

/// The type of network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterfaceKind {
    /// Bridge interface connecting multiple networks.
    Bridge,
    /// Standard Ethernet interface.
    Ethernet,
}

/// An IPv4 address with a CIDR prefix length, serialized as `"a.b.c.d/prefix"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr4 {
    /// IPv4 address component.
    pub address: Ipv4Addr,
    /// CIDR prefix length (0–32).
    pub prefix: u8,
}

impl FromStr for Cidr4 {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (addr_part, prefix_part) = s
            .split_once('/')
            .ok_or_else(|| format!("missing '/' in CIDR address: '{}'", s))?;
        let address = addr_part
            .parse::<Ipv4Addr>()
            .map_err(|e| format!("invalid IPv4 address '{}': {}", addr_part, e))?;
        let prefix = prefix_part
            .parse::<u8>()
            .map_err(|e| format!("invalid prefix length '{}': {}", prefix_part, e))?;
        if prefix > 32 {
            return Err(format!("prefix length {} exceeds 32", prefix));
        }
        Ok(Self { address, prefix })
    }
}

impl fmt::Display for Cidr4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

impl Serialize for Cidr4 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Cidr4 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<Cidr4>().map_err(serde::de::Error::custom)
    }
}

/// An IPv6 address with a CIDR prefix length, serialized as `"addr/prefix"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr6 {
    /// IPv6 address component.
    pub address: Ipv6Addr,
    /// CIDR prefix length (0–128).
    pub prefix: u8,
}

impl FromStr for Cidr6 {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (addr_part, prefix_part) = s
            .split_once('/')
            .ok_or_else(|| format!("missing '/' in CIDR address: '{}'", s))?;
        let address = addr_part
            .parse::<Ipv6Addr>()
            .map_err(|e| format!("invalid IPv6 address '{}': {}", addr_part, e))?;
        let prefix = prefix_part
            .parse::<u8>()
            .map_err(|e| format!("invalid prefix length '{}': {}", prefix_part, e))?;
        if prefix > 128 {
            return Err(format!("prefix length {} exceeds 128", prefix));
        }
        Ok(Self { address, prefix })
    }
}

impl fmt::Display for Cidr6 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

impl Serialize for Cidr6 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Cidr6 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<Cidr6>().map_err(serde::de::Error::custom)
    }
}

/// IPv4 configuration for a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Ipv4InterfaceConfig {
    /// Whether to use DHCP for IPv4 address assignment.
    pub dhcp: bool,
    /// Static IPv4 addresses with CIDR notation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<Cidr4>,
    /// Optional IPv4 gateway address.
    pub gateway: Option<Ipv4Addr>,
}

/// IPv6 configuration for a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Ipv6InterfaceConfig {
    /// Whether to use SLAAC for IPv6 address assignment.
    pub autoconf: bool,
    /// Static IPv6 addresses with CIDR notation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<Cidr6>,
    /// Optional IPv6 gateway address.
    pub gateway: Option<Ipv6Addr>,
}

/// Bridge-specific configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BridgeConfig {
    /// Member ports attached to the bridge.
    pub port: Vec<String>,
    /// Whether Spanning Tree Protocol is enabled.
    pub stp: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Codec, TomlCodec};
    use crate::system::SystemConfig;

    #[test]
    fn cidr4_parse_valid() {
        // ARRANGE / ACT
        let cidr: Cidr4 = "192.168.1.10/24".parse().unwrap();

        // ASSERT
        assert_eq!(cidr.address, "192.168.1.10".parse::<Ipv4Addr>().unwrap());
        assert_eq!(cidr.prefix, 24);
    }

    #[test]
    fn cidr4_parse_prefix_zero() {
        let cidr: Cidr4 = "0.0.0.0/0".parse().unwrap();
        assert_eq!(cidr.prefix, 0);
    }

    #[test]
    fn cidr4_parse_prefix_32() {
        let cidr: Cidr4 = "10.0.0.1/32".parse().unwrap();
        assert_eq!(cidr.prefix, 32);
    }

    #[test]
    fn cidr4_parse_rejects_missing_slash() {
        assert!("192.168.1.1".parse::<Cidr4>().is_err());
    }

    #[test]
    fn cidr4_parse_rejects_invalid_ip() {
        assert!("999.0.0.1/24".parse::<Cidr4>().is_err());
    }

    #[test]
    fn cidr4_parse_rejects_prefix_over_32() {
        assert!("192.168.1.1/33".parse::<Cidr4>().is_err());
    }

    #[test]
    fn cidr4_display_round_trip() {
        // ARRANGE
        let input = "10.0.0.5/16";

        // ACT
        let cidr: Cidr4 = input.parse().unwrap();

        // ASSERT
        assert_eq!(cidr.to_string(), input);
    }

    #[test]
    fn cidr4_serde_round_trip() {
        // ARRANGE
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            cidr: Cidr4,
        }
        let wrapper = Wrapper {
            cidr: Cidr4 {
                address: "192.168.0.1".parse().unwrap(),
                prefix: 24,
            },
        };

        // ACT
        let serialized = toml::to_string(&wrapper).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();

        // ASSERT
        assert_eq!(wrapper.cidr, deserialized.cidr);
    }

    #[test]
    fn network_config_dns_valid_populates_split_lists() {
        // ARRANGE
        let toml_str = r#"
[network]
dns = ["9.9.9.9", "2620:fe::fe"]
"#;

        // ACT
        let config: SystemConfig = TomlCodec::decode(toml_str).unwrap();
        let v4: Vec<Ipv4Addr> = config.network.ipv4_dns().collect();
        let v6: Vec<Ipv6Addr> = config.network.ipv6_dns().collect();

        // ASSERT
        assert_eq!(v4.len(), 1);
        assert_eq!(v6.len(), 1);
        assert_eq!(v4[0], "9.9.9.9".parse::<Ipv4Addr>().unwrap());
    }

    #[test]
    fn network_config_dns_invalid_rejects_at_deserialization() {
        // ARRANGE
        let toml_str = r#"
[network]
dns = ["not-an-ip"]
"#;

        // ACT & ASSERT
        assert!(TomlCodec::decode::<SystemConfig>(toml_str).is_err());
    }

    #[test]
    fn multiple_addresses_deserialization() {
        // ARRANGE
        let toml_str = r#"
[[network.interfaces]]
name = "eth0"
type = "ethernet"
ipv4.addresses = ["192.168.1.10/24", "10.0.0.1/8"]
ipv4.gateway = "192.168.1.1"
"#;

        // ACT
        let config: SystemConfig = TomlCodec::decode(toml_str).unwrap();
        let iface = &config.network.interfaces[0];
        let ipv4 = iface.ipv4.as_ref().unwrap();

        // ASSERT
        assert_eq!(ipv4.addresses.len(), 2);
        assert_eq!(ipv4.addresses[0].to_string(), "192.168.1.10/24");
        assert_eq!(ipv4.addresses[1].to_string(), "10.0.0.1/8");
        assert_eq!(
            ipv4.gateway,
            Some("192.168.1.1".parse::<Ipv4Addr>().unwrap())
        );
    }

    #[test]
    fn dhcp_interface_has_empty_addresses_by_default() {
        // ARRANGE
        let toml_str = r#"
[[network.interfaces]]
name = "eth0"
type = "ethernet"
ipv4.dhcp = true
"#;

        // ACT
        let config: SystemConfig = TomlCodec::decode(toml_str).unwrap();
        let ipv4 = config.network.interfaces[0].ipv4.as_ref().unwrap();

        // ASSERT
        assert!(ipv4.dhcp);
        assert!(ipv4.addresses.is_empty());
        assert!(ipv4.gateway.is_none());
    }

    #[test]
    fn cidr6_parse_valid() {
        // ARRANGE / ACT
        let cidr: Cidr6 = "2001:db8::1/64".parse().unwrap();

        // ASSERT
        assert_eq!(cidr.address, "2001:db8::1".parse::<Ipv6Addr>().unwrap());
        assert_eq!(cidr.prefix, 64);
    }

    #[test]
    fn cidr6_parse_prefix_zero() {
        let cidr: Cidr6 = "::/0".parse().unwrap();
        assert_eq!(cidr.prefix, 0);
    }

    #[test]
    fn cidr6_parse_prefix_128() {
        let cidr: Cidr6 = "::1/128".parse().unwrap();
        assert_eq!(cidr.prefix, 128);
    }

    #[test]
    fn cidr6_parse_rejects_missing_slash() {
        assert!("2001:db8::1".parse::<Cidr6>().is_err());
    }

    #[test]
    fn cidr6_parse_rejects_invalid_ip() {
        assert!("gggg::1/64".parse::<Cidr6>().is_err());
    }

    #[test]
    fn cidr6_parse_rejects_prefix_over_128() {
        assert!("::1/129".parse::<Cidr6>().is_err());
    }

    #[test]
    fn cidr6_display_round_trip() {
        // ARRANGE
        let input = "2001:db8::1/64";

        // ACT
        let cidr: Cidr6 = input.parse().unwrap();

        // ASSERT
        assert_eq!(cidr.to_string(), input);
    }

    #[test]
    fn cidr6_serde_round_trip() {
        // ARRANGE
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            cidr: Cidr6,
        }
        let wrapper = Wrapper {
            cidr: Cidr6 {
                address: "2001:db8::1".parse().unwrap(),
                prefix: 64,
            },
        };

        // ACT
        let serialized = toml::to_string(&wrapper).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();

        // ASSERT
        assert_eq!(wrapper.cidr, deserialized.cidr);
    }

    #[test]
    fn static_ipv6_interface_deserialization() {
        // ARRANGE
        let toml_str = r#"
[[network.interfaces]]
name = "eth0"
type = "ethernet"
ipv6.addresses = ["2001:db8::1/64", "2001:db8::2/64"]
ipv6.gateway = "2001:db8::1"
"#;

        // ACT
        let config: SystemConfig = TomlCodec::decode(toml_str).unwrap();
        let iface = &config.network.interfaces[0];
        let ipv6 = iface.ipv6.as_ref().unwrap();

        // ASSERT
        assert_eq!(ipv6.addresses.len(), 2);
        assert_eq!(ipv6.addresses[0].to_string(), "2001:db8::1/64");
        assert_eq!(ipv6.addresses[1].to_string(), "2001:db8::2/64");
        assert_eq!(
            ipv6.gateway,
            Some("2001:db8::1".parse::<Ipv6Addr>().unwrap())
        );
        assert!(!ipv6.autoconf);
    }

    #[test]
    fn auto_ethernet_interface_deserialization() {
        // ARRANGE
        let toml_str = r#"
[[network.interfaces]]
name = "auto"
type = "ethernet"
ipv4.dhcp = true
ipv6.autoconf = true
"#;

        // ACT
        let config: SystemConfig = TomlCodec::decode(toml_str).unwrap();
        let iface = &config.network.interfaces[0];

        // ASSERT
        assert_eq!(iface.name, "auto");
        assert_eq!(iface.kind, InterfaceKind::Ethernet);
        assert!(iface.ipv4.as_ref().unwrap().dhcp);
        assert!(iface.ipv6.as_ref().unwrap().autoconf);
    }
}
