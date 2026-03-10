//! Immutable host system configuration.

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
pub const CONFIG_EXTENSION: &str = "toml";

pub(crate) static CONFIG: OnceLock<SystemConfig> = OnceLock::new();

const DEFAULT_CONFIG: &str = include_str!("../../../default.toml");

/// Top-level system configuration covering host, disk, network, and VM settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
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
#[serde(default)]
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
#[serde(default)]
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
#[serde(default)]
pub struct NetworkConfig {
    pub ipv6: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

/// Serializes a [`SystemConfig`] to a TOML string.
pub fn serialize(config: &SystemConfig) -> Result<String> {
    toml::to_string_pretty(config).map_err(Into::into)
}

/// Serializes the default system configuration to a TOML string.
pub fn serialize_default() -> String {
    toml::to_string_pretty(&SystemConfig::default()).expect("Failed to serialize default config")
}

/// Parses a [`SystemConfig`] from a TOML string, validating it.
pub fn parse_from_str(contents: &str) -> Result<SystemConfig> {
    let config: SystemConfig = toml::from_str(contents)?;
    config.validate()?;
    Ok(config)
}

/// Loads system config from a file, falling back to defaults if not found.
pub fn load_from_path(path: &Path) -> Result<SystemConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(Into::into)
    } else {
        toml::from_str(DEFAULT_CONFIG).map_err(Into::into)
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
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: SystemConfig = toml::from_str(&serialized).unwrap();

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
        let result = toml::from_str::<SystemConfig>("invalid toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_default() {
        // ACT
        let default_str = toml::to_string_pretty(&SystemConfig::default()).unwrap();
        let config: SystemConfig = toml::from_str(&default_str).unwrap();

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
    fn test_config_ignores_unknown_sections() {
        // ARRANGE
        let toml = r#"
[host]
name = "muak"
port = 50051

[auth]
revoked = []

[[auth.users]]
fingerprint = "abc"
permissions = ["admin"]
"#;

        // ACT
        let config: SystemConfig = toml::from_str(toml).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "muak");
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
}
