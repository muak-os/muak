use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub ipv6: bool,
    pub dns: Vec<String>,
    pub interfaces: Vec<InterfaceConfig>,
}

impl NetworkConfig {
    /// Validates that all entries in `dns` are parseable IP addresses.
    pub fn validate_dns(&self) -> Result<()> {
        let invalid = self.dns.iter().find(|e| e.parse::<IpAddr>().is_err());
        if let Some(entry) = invalid {
            return Err(ConfigError::ValidationError(format!(
                "network.dns contains invalid IP address: '{}'",
                entry
            )));
        }
        Ok(())
    }
}

/// Declarative configuration for a single network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: InterfaceKind,
    #[serde(default)]
    pub ipv4: Option<Ipv4InterfaceConfig>,
    #[serde(default)]
    pub ipv6: Option<Ipv6InterfaceConfig>,
    #[serde(default)]
    pub bridge: Option<BridgeConfig>,
}

/// The type of network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterfaceKind {
    Bridge,
    Ethernet,
}

/// An IPv4 address with a CIDR prefix length, serialized as `"a.b.c.d/prefix"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr4 {
    pub address: Ipv4Addr,
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

/// IPv4 configuration for a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Ipv4InterfaceConfig {
    pub dhcp: bool,
    pub addresses: Vec<Cidr4>,
    pub gateway: Option<Ipv4Addr>,
}

/// IPv6 configuration for a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Ipv6InterfaceConfig {
    pub autoconf: bool,
}

/// Bridge-specific configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BridgeConfig {
    pub port: Vec<String>,
    pub stp: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Codec, TomlCodec};
    use crate::system::SystemConfig;

    #[test]
    fn test_cidr4_parse_valid() {
        // ARRANGE / ACT
        let cidr: Cidr4 = "192.168.1.10/24".parse().unwrap();

        // ASSERT
        assert_eq!(cidr.address, "192.168.1.10".parse::<Ipv4Addr>().unwrap());
        assert_eq!(cidr.prefix, 24);
    }

    #[test]
    fn test_cidr4_parse_prefix_zero() {
        let cidr: Cidr4 = "0.0.0.0/0".parse().unwrap();
        assert_eq!(cidr.prefix, 0);
    }

    #[test]
    fn test_cidr4_parse_prefix_32() {
        let cidr: Cidr4 = "10.0.0.1/32".parse().unwrap();
        assert_eq!(cidr.prefix, 32);
    }

    #[test]
    fn test_cidr4_parse_rejects_missing_slash() {
        assert!("192.168.1.1".parse::<Cidr4>().is_err());
    }

    #[test]
    fn test_cidr4_parse_rejects_invalid_ip() {
        assert!("999.0.0.1/24".parse::<Cidr4>().is_err());
    }

    #[test]
    fn test_cidr4_parse_rejects_prefix_over_32() {
        assert!("192.168.1.1/33".parse::<Cidr4>().is_err());
    }

    #[test]
    fn test_cidr4_display_round_trip() {
        // ARRANGE
        let input = "10.0.0.5/16";

        // ACT
        let cidr: Cidr4 = input.parse().unwrap();

        // ASSERT
        assert_eq!(cidr.to_string(), input);
    }

    #[test]
    fn test_cidr4_serde_round_trip() {
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
    fn test_validate_dns_valid() {
        // ARRANGE
        let cfg = NetworkConfig {
            dns: vec!["9.9.9.9".to_string(), "2620:fe::fe".to_string()],
            ..Default::default()
        };

        // ACT & ASSERT
        assert!(cfg.validate_dns().is_ok());
    }

    #[test]
    fn test_validate_dns_invalid() {
        // ARRANGE
        let cfg = NetworkConfig {
            dns: vec!["not-an-ip".to_string()],
            ..Default::default()
        };

        // ACT & ASSERT
        assert!(cfg.validate_dns().is_err());
    }

    #[test]
    fn test_multiple_addresses_deserialization() {
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
    fn test_dhcp_interface_has_empty_addresses_by_default() {
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
}
