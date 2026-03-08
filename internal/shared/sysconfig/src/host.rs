//! Immutable host system configuration.

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
pub const CONFIG_EXTENSION: &str = "toml";

pub(crate) static CONFIG: OnceLock<HostConfig> = OnceLock::new();

const DEFAULT_CONFIG: &str = include_str!("../../../default.toml");

/// Top-level host configuration covering system, network, and VM settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct HostConfig {
    pub system: SystemConfig,
    pub network: NetworkConfig,
    pub vm: VmConfig,
}

impl HostConfig {
    /// Validates that required fields are present and sensible.
    pub fn validate(&self) -> Result<()> {
        if self.system.name.is_empty() {
            return Err(ConfigError::ValidationError(
                "system.name must be specified".to_string(),
            ));
        }
        if self.system.port == 0 {
            return Err(ConfigError::ValidationError(
                "system.port must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates that the config is complete enough for installation.
    pub fn validate_for_install(&self) -> Result<()> {
        self.validate()?;
        if self.system.disk.is_empty() {
            return Err(ConfigError::ValidationError(
                "system.disk must be specified for installation".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates that the config is acceptable for an update operation.
    pub fn validate_for_update(&self, installed: &HostConfig) -> Result<()> {
        self.validate()?;
        if self.system.disk != installed.system.disk {
            return Err(ConfigError::ValidationError(format!(
                "system.disk cannot be changed after install (installed: '{}', requested: '{}')",
                installed.system.disk, self.system.disk
            )));
        }
        if installed.system.secureboot && !self.system.secureboot {
            return Err(ConfigError::ValidationError(
                "system.secureboot cannot be disabled after Secure Boot keys have been enrolled"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

include!(concat!(env!("OUT_DIR"), "/defaults.rs"));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    pub name: String,
    pub disk: String,
    pub image: String,
    pub extensions: Vec<String>,
    pub secureboot: bool,
    pub port: u16,
    pub ntp: String,
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

/// Serializes a [`HostConfig`] to a TOML string.
pub fn serialize(config: &HostConfig) -> Result<String> {
    toml::to_string_pretty(config).map_err(Into::into)
}

/// Serializes the default host configuration to a TOML string.
pub fn serialize_default() -> String {
    toml::to_string_pretty(&HostConfig::default()).expect("Failed to serialize default config")
}

/// Parses a [`HostConfig`] from a TOML string, validating it.
pub fn parse_from_str(contents: &str) -> Result<HostConfig> {
    let config: HostConfig = toml::from_str(contents)?;
    config.validate()?;
    Ok(config)
}

/// Loads host config from a file, falling back to defaults if not found.
pub fn load_from_path(path: &Path) -> Result<HostConfig> {
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
        let config = HostConfig::default();

        // ACT
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: HostConfig = toml::from_str(&serialized).unwrap();

        // ASSERT
        assert_eq!(config.system.port, deserialized.system.port);
    }

    #[test]
    fn test_validation_success() {
        // ARRANGE
        let mut config = HostConfig::default();
        config.system.port = 8080;
        config.system.disk = "test".to_string();

        // ACT & ASSERT
        assert!(config.validate().is_ok());
        assert!(config.validate_for_install().is_ok());
    }

    #[test]
    fn test_validation_failure_port_zero() {
        // ARRANGE
        let mut config = HostConfig::default();
        config.system.port = 0;

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_failure_empty_disk_install() {
        // ARRANGE
        let mut config = HostConfig::default();
        config.system.port = 8080;
        config.system.disk = "".to_string();

        // ACT & ASSERT
        assert!(config.validate_for_install().is_err());
    }

    #[test]
    fn test_parse_from_str_invalid_toml() {
        // ACT & ASSERT
        let result = toml::from_str::<HostConfig>("invalid toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_default() {
        // ACT
        let default_str = toml::to_string_pretty(&HostConfig::default()).unwrap();
        let config: HostConfig = toml::from_str(&default_str).unwrap();

        // ASSERT
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_for_update_rejects_disk_change() {
        // ARRANGE
        let mut installed = HostConfig::default();
        installed.system.port = 8080;
        installed.system.disk = "/dev/sda".to_string();

        let mut requested = installed.clone();
        requested.system.disk = "/dev/sdb".to_string();

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_false_to_true() {
        // ARRANGE
        let mut installed = HostConfig::default();
        installed.system.port = 8080;
        installed.system.secureboot = false;

        let mut requested = installed.clone();
        requested.system.secureboot = true;

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_validate_for_update_rejects_secureboot_true_to_false() {
        // ARRANGE
        let mut installed = HostConfig::default();
        installed.system.port = 8080;
        installed.system.secureboot = true;

        let mut requested = installed.clone();
        requested.system.secureboot = false;

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_unchanged_false() {
        // ARRANGE
        let mut installed = HostConfig::default();
        installed.system.port = 8080;
        installed.system.secureboot = false;

        // ACT & ASSERT
        assert!(installed.clone().validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_validate_for_update_allows_secureboot_unchanged_true() {
        // ARRANGE
        let mut installed = HostConfig::default();
        installed.system.port = 8080;
        installed.system.secureboot = true;

        // ACT & ASSERT
        assert!(installed.clone().validate_for_update(&installed).is_ok());
    }

    #[test]
    fn test_config_ignores_unknown_sections() {
        // ARRANGE
        let toml = r#"
[system]
name = "muak"
port = 50051

[auth]
revoked = []

[[auth.users]]
fingerprint = "abc"
permissions = ["admin"]
"#;

        // ACT
        let config: HostConfig = toml::from_str(toml).unwrap();

        // ASSERT
        assert_eq!(config.system.name, "muak");
    }
}
