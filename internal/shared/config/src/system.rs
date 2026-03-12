//! Immutable host system configuration.

use std::net::IpAddr;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::codec::{Codec, TomlCodec};
use crate::error::{ConfigError, Result};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
pub const CONFIG_EXTENSION: &str = "toml";

pub(crate) static CONFIG: OnceLock<SystemConfig> = OnceLock::new();

const DEFAULT_CONFIG: &str = include_str!("../../../default.toml");

/// Top-level system configuration covering host, disk, network, and VM settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    pub host: HostConfig,
    pub disk: DiskConfig,
    pub network: NetworkConfig,
    pub vm: VmConfig,
}

impl SystemConfig {
    /// Validates that required fields are present and sensible.
    pub fn validate(&self) -> Result<()> {
        if self.host.name.is_empty() {
            return Err(ConfigError::ValidationError(
                "host.name must be specified".to_string(),
            ));
        }
        if self.host.port == 0 {
            return Err(ConfigError::ValidationError(
                "host.port must be greater than 0".to_string(),
            ));
        }
        self.network.validate_dns()?;
        Ok(())
    }

    /// Validates that the config is complete enough for installation.
    pub fn validate_for_install(&self) -> Result<()> {
        self.validate()?;
        if self.disk.system.is_empty() {
            return Err(ConfigError::ValidationError(
                "disk.system must be specified for installation".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates that the config is acceptable for an update operation.
    pub fn validate_for_update(&self, installed: &SystemConfig) -> Result<()> {
        self.validate()?;
        if self.disk.system != installed.disk.system {
            return Err(ConfigError::ValidationError(format!(
                "disk.system cannot be changed after install (installed: '{}', requested: '{}')",
                installed.disk.system, self.disk.system
            )));
        }
        if self.disk.data_disk() != installed.disk.data_disk() {
            return Err(ConfigError::ValidationError(format!(
                "disk.data cannot be changed after install (installed: '{}', requested: '{}')",
                installed.disk.data_disk(),
                self.disk.data_disk()
            )));
        }
        if installed.host.secureboot && !self.host.secureboot {
            return Err(ConfigError::ValidationError(
                "host.secureboot cannot be disabled after Secure Boot keys have been enrolled"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

include!(concat!(env!("OUT_DIR"), "/defaults.rs"));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostConfig {
    pub name: String,
    pub image: String,
    pub extensions: Vec<String>,
    pub secureboot: bool,
    pub port: u16,
    pub ntp: String,
}

/// Disk assignment configuration for system and data partitions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DiskConfig {
    pub system: String,
    pub data: Option<String>,
}

impl DiskConfig {
    /// Returns the effective data disk path, falling back to `system` when `data` is unset.
    pub fn data_disk(&self) -> &str {
        match &self.data {
            Some(d) if !d.is_empty() => d.as_str(),
            _ => &self.system,
        }
    }

    /// Returns true when the data partition lives on a separate physical disk.
    pub fn is_split(&self) -> bool {
        matches!(&self.data, Some(d) if !d.is_empty() && d != &self.system)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// IPv4 configuration for a network interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Ipv4InterfaceConfig {
    pub dhcp: bool,
    pub address: Option<std::net::Ipv4Addr>,
    pub prefix: Option<u8>,
    pub gateway: Option<std::net::Ipv4Addr>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VmConfig {
    pub auto_restart: bool,
}

/// Initializes the host config.
pub fn init() -> Result<()> {
    let config = load_from_path(Path::new(CONFIG_PATH))?;
    config.validate()?;
    CONFIG
        .set(config)
        .map_err(|_| ConfigError::AlreadyInitialized)?;
    Ok(())
}

/// Serializes a [`SystemConfig`] to a string.
pub fn serialize(config: &SystemConfig) -> Result<String> {
    TomlCodec::encode(config)
}

/// Serializes the default system configuration to a string.
pub fn serialize_default() -> String {
    TomlCodec::encode(&SystemConfig::default()).expect("Failed to serialize default config")
}

/// Parses a [`SystemConfig`] from a string, validating it.
pub fn parse_from_str(contents: &str) -> Result<SystemConfig> {
    let config: SystemConfig = TomlCodec::decode(contents)?;
    config.validate()?;
    Ok(config)
}

/// Diffs two config strings, returning `(field_path, before, after)` for each changed field.
pub fn diff(a: &str, b: &str) -> Result<Vec<(String, String, String)>> {
    let a = TomlCodec::decode(a)?;
    let b = TomlCodec::decode(b)?;
    let mut changes = Vec::new();
    diff_values(&mut changes, "", &a, &b);
    Ok(changes)
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{}.{}", prefix, key)
    }
}

fn diff_values(
    changes: &mut Vec<(String, String, String)>,
    prefix: &str,
    a: &toml::Value,
    b: &toml::Value,
) {
    let (toml::Value::Table(fa), toml::Value::Table(fb)) = (a, b) else {
        if a != b {
            changes.push((prefix.to_string(), a.to_string(), b.to_string()));
        }
        return;
    };
    for (key, va) in fa {
        let path = join_path(prefix, key);
        match fb.get(key) {
            Some(vb) => diff_values(changes, &path, va, vb),
            None => changes.push((path, va.to_string(), String::new())),
        }
    }
    for (key, vb) in fb.iter().filter(|(k, _)| !fa.contains_key(*k)) {
        changes.push((join_path(prefix, key), String::new(), vb.to_string()));
    }
}

/// Loads system config from a file, falling back to defaults if not found.
pub fn load_from_path(path: &Path) -> Result<SystemConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        TomlCodec::decode(&contents)
    } else {
        TomlCodec::decode(DEFAULT_CONFIG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_config_serialization() {
        // ARRANGE
        let config = SystemConfig::default();

        // ACT
        let serialized = TomlCodec::encode(&config).unwrap();
        let deserialized: SystemConfig = TomlCodec::decode(&serialized).unwrap();

        // ASSERT
        assert_eq!(config.host.port, deserialized.host.port);
    }

    #[test]
    fn test_validation_success() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.port = 8080;
        config.disk.system = "/dev/sda".to_string();

        // ACT & ASSERT
        assert!(config.validate().is_ok());
        assert!(config.validate_for_install().is_ok());
    }

    #[test]
    fn test_validation_failure_port_zero() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.port = 0;

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_failure_empty_disk_install() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.port = 8080;
        config.disk.system = String::new();

        // ACT & ASSERT
        assert!(config.validate_for_install().is_err());
    }

    #[test]
    fn test_parse_from_str_invalid_toml() {
        // ACT & ASSERT
        let result = TomlCodec::decode::<SystemConfig>("invalid toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_default() {
        // ACT
        let default_str = TomlCodec::encode(&SystemConfig::default()).unwrap();
        let config: SystemConfig = TomlCodec::decode(&default_str).unwrap();

        // ASSERT
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_for_update_rejects_system_disk_change() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.disk.system = "/dev/sda".to_string();

        let mut requested = installed.clone();
        requested.disk.system = "/dev/sdb".to_string();

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn test_validate_for_update_rejects_data_disk_change() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.disk.system = "/dev/sda".to_string();
        installed.disk.data = Some("/dev/sdb".to_string());

        let mut requested = installed.clone();
        requested.disk.data = Some("/dev/sdc".to_string());

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_false_to_true() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.host.secureboot = false;

        let mut requested = installed.clone();
        requested.host.secureboot = true;

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_validate_for_update_rejects_secureboot_true_to_false() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.host.secureboot = true;

        let mut requested = installed.clone();
        requested.host.secureboot = false;

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_unchanged_false() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.host.secureboot = false;

        // ACT & ASSERT
        assert!(installed.clone().validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_unchanged_true() {
        // ARRANGE
        let mut installed = SystemConfig::default();
        installed.host.port = 8080;
        installed.host.secureboot = true;

        // ACT & ASSERT
        assert!(installed.clone().validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_config_rejects_unknown_sections() {
        // ARRANGE
        let toml_str = r#"
[host]
name = "muak"
port = 50051

[system]
image = "192.168.100.1:5000/installer:latest"
"#;

        // ACT
        let result: Result<SystemConfig> = TomlCodec::decode(toml_str);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_disk_config_data_disk_fallback() {
        // ARRANGE
        let mut disk = DiskConfig::default();
        disk.system = "/dev/sda".to_string();

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sda");
        assert!(!disk.is_split());
    }

    #[test]
    fn test_disk_config_split() {
        // ARRANGE
        let disk = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some("/dev/sdb".to_string()),
        };

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sdb");
        assert!(disk.is_split());
    }

    #[test]
    fn test_validation_failure_empty_name() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.name = String::new();
        config.host.port = 8080;

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_serialize_round_trip() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.port = 9090;
        config.host.name = "testhost".to_string();
        config.disk.system = "/dev/nvme0n1".to_string();
        config.network.ipv6 = true;
        config.network.dns = vec!["9.9.9.9".to_string()];
        config.vm.auto_restart = true;

        // ACT
        let s = serialize(&config).unwrap();
        let restored: SystemConfig = TomlCodec::decode(&s).unwrap();

        // ASSERT
        assert_eq!(restored.host.port, 9090);
        assert_eq!(restored.host.name, "testhost");
        assert_eq!(restored.disk.system, "/dev/nvme0n1");
        assert!(restored.network.ipv6);
        assert_eq!(restored.network.dns, vec!["9.9.9.9"]);
        assert!(restored.vm.auto_restart);
    }

    #[test]
    fn test_parse_from_str_valid() {
        // ARRANGE
        let toml_str = r#"
[host]
name = "myhost"
port = 1234
"#;

        // ACT
        let config = parse_from_str(toml_str).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "myhost");
        assert_eq!(config.host.port, 1234);
    }

    #[test]
    fn test_parse_from_str_invalid_format_error() {
        // ACT
        let result = parse_from_str("[[[ invalid");

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_from_str_validation_error() {
        // ARRANGE
        let str = "[host]\nport = 0\n";

        // ACT
        let result = parse_from_str(str);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_load_from_path_existing_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "[host]\nname = \"loaded\"\nport = 7777\n";
        std::fs::write(&path, content).unwrap();

        // ACT
        let config = load_from_path(&path).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "loaded");
        assert_eq!(config.host.port, 7777);
    }

    #[test]
    fn test_load_from_path_nonexistent_uses_default() {
        // ARRANGE
        let path = std::path::Path::new("/nonexistent/config.toml");

        // ACT
        let config = load_from_path(path).unwrap();

        // ASSERT
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_disk_config_empty_data_string_falls_back_to_system() {
        // ARRANGE
        let disk = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some(String::new()),
        };

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sda");
        assert!(!disk.is_split());
    }

    #[test]
    fn test_diff_no_changes() {
        // ARRANGE
        let config = "[host]\nname = \"x\"\nport = 1\n";

        // ACT
        let changes = diff(config, config).unwrap();

        // ASSERT
        assert!(changes.is_empty());
    }

    #[test]
    fn test_diff_changed_scalar() {
        // ARRANGE
        let a = "[host]\nname = \"alpha\"\nport = 8080\n";
        let b = "[host]\nname = \"beta\"\nport = 8080\n";

        // ACT
        let changes = diff(a, b).unwrap();

        // ASSERT
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "host.name");
        assert!(changes[0].1.contains("alpha"));
        assert!(changes[0].2.contains("beta"));
    }

    #[test]
    fn test_diff_added_key() {
        // ARRANGE
        let a = "[host]\nport = 1\n\n[network]\nipv6 = false\n";
        let b = "[host]\nport = 1\n\n[network]\nipv6 = true\n";

        // ACT
        let changes = diff(a, b).unwrap();

        // ASSERT
        let found = changes.iter().any(|(k, before, after)| {
            k == "network.ipv6" && before.contains("false") && after.contains("true")
        });
        assert!(found, "expected network.ipv6 change, got: {:?}", changes);
    }

    #[test]
    fn test_diff_removed_key() {
        // ARRANGE
        let a = "[host]\nport = 1\n\n[network]\nipv6 = true\n";
        let b = "[host]\nport = 1\n\n[network]\nipv6 = false\n";

        // ACT
        let changes = diff(a, b).unwrap();

        // ASSERT
        let found = changes.iter().any(|(k, before, after)| {
            k == "network.ipv6" && before.contains("true") && after.contains("false")
        });
        assert!(found, "expected network.ipv6 change, got: {:?}", changes);
    }

    #[test]
    fn test_diff_whole_section_added() {
        // ARRANGE
        let a = "[host]\nport = 1\n";
        let b = "[host]\nport = 1\n\n[network]\nipv6 = true\n";

        // ACT
        let changes = diff(a, b).unwrap();

        // ASSERT
        let found = changes
            .iter()
            .any(|(k, before, _after)| k == "network" && before.is_empty());
        assert!(found, "expected network section added, got: {:?}", changes);
    }

    #[test]
    fn test_diff_whole_section_removed() {
        // ARRANGE
        let a = "[host]\nport = 1\n\n[network]\nipv6 = true\n";
        let b = "[host]\nport = 1\n";

        // ACT
        let changes = diff(a, b).unwrap();

        // ASSERT
        let found = changes
            .iter()
            .any(|(k, _before, after)| k == "network" && after.is_empty());
        assert!(
            found,
            "expected network section removed, got: {:?}",
            changes
        );
    }

    #[test]
    fn test_host_config_fields() {
        // ARRANGE
        let mut config = SystemConfig::default();
        config.host.image = "myimage".to_string();
        config.host.extensions = vec!["ext1".to_string()];
        config.host.ntp = "pool.ntp.org".to_string();
        config.host.secureboot = true;

        // ACT
        let s = serialize(&config).unwrap();
        let restored: SystemConfig = TomlCodec::decode(&s).unwrap();

        // ASSERT
        assert_eq!(restored.host.image, "myimage");
        assert_eq!(restored.host.extensions, vec!["ext1"]);
        assert_eq!(restored.host.ntp, "pool.ntp.org");
        assert!(restored.host.secureboot);
    }
}
