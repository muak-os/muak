//! # sysconfig
//!
//! A Rust crate for managing configuration settings in a Muak-based system.
//!
//! This crate provides a structured way to load, validate, and access configuration
//! data from TOML files. It uses a global static configuration pattern for runtime
//! access, ensuring thread-safe, once-initialized state.
//!
//! Configurations are loaded from `/run/state/config.toml` at runtime, falling back
//! to embedded defaults if the file does not exist. The crate supports serialization,
//! validation, and global access to host-level settings for system, network, and VM components.
//!
//! ## Example
//!
//! ```rust
//! use sysconfig::{init, system};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize the global config (must be called before access)
//!     init()?;
//!
//!     // Access system config
//!     let disk = &system().disk;
//!     Ok(())
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

mod error;
pub mod permission;

pub use error::{ConfigError, Result};
pub use permission::Permission;

pub const CONFIG_PATH: &str = "/run/state/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../../../default.toml");

static CONFIG: OnceLock<HostConfig> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct HostConfig {
    pub system: SystemConfig,
    pub network: NetworkConfig,
    pub vm: VmConfig,
    pub auth: AuthConfig,
}

impl HostConfig {
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

    pub fn validate_for_install(&self) -> Result<()> {
        self.validate()?;
        if self.system.disk.is_empty() {
            return Err(ConfigError::ValidationError(
                "system.disk must be specified for installation".to_string(),
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
    pub port: u16,
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

/// Authentication and authorization configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    pub users: Vec<AuthUser>,
    pub revoked: Vec<String>,
}

/// An authorized user identified by certificate fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub fingerprint: String,
    pub permissions: Vec<Permission>,
}

/// Initializes the global configuration.
///
/// Loads the config from `CONFIG_PATH`, validates it, and stores it globally.
/// Must be called before accessing config via other functions.
/// Returns an error if already initialized or if loading/validation fails.
pub fn init() -> Result<()> {
    let config = load_from_path(Path::new(CONFIG_PATH))?;
    config.validate()?;
    CONFIG
        .set(config)
        .map_err(|_| ConfigError::AlreadyInitialized)
}

/// Returns a reference to the global system configuration.
///
/// # Panics
///
/// Panics if `init()` has not been called.
pub fn system() -> &'static SystemConfig {
    &config().system
}

/// Returns a reference to the global network configuration.
///
/// # Panics
///
/// Panics if `init()` has not been called.
pub fn network() -> &'static NetworkConfig {
    &config().network
}

/// Returns a reference to the global VM configuration.
///
/// # Panics
///
/// Panics if `init()` has not been called.
pub fn vm() -> &'static VmConfig {
    &config().vm
}

/// Returns a reference to the global auth configuration.
///
/// # Panics
///
/// Panics if `init()` has not been called.
pub fn auth() -> &'static AuthConfig {
    &config().auth
}

/// Returns a reference to the global host configuration.
///
/// # Panics
///
/// Panics if `init()` has not been called.
pub fn config() -> &'static HostConfig {
    CONFIG.get().expect("Config not initialized")
}

/// Attempts to return a reference to the global host configuration.
///
/// Returns `None` if `init()` has not been called.
pub fn try_config() -> Option<&'static HostConfig> {
    CONFIG.get()
}

/// Serializes the default system configuration to a TOML string.
pub fn serialize_default() -> String {
    toml::to_string_pretty(&HostConfig::default()).expect("Failed to serialize default config")
}

/// Serializes a system configuration to a TOML string.
pub fn serialize(config: &HostConfig) -> Result<String> {
    toml::to_string_pretty(config).map_err(Into::into)
}

/// Parses a system configuration from a TOML string and validates it.
pub fn parse_from_str(contents: &str) -> Result<HostConfig> {
    let config: HostConfig = toml::from_str(contents)?;
    config.validate()?;
    Ok(config)
}

/// Loads configuration from a filepath, falling back to defaults if not found.
pub(crate) fn load_from_path(path: &Path) -> Result<HostConfig> {
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
        let config = HostConfig::default();
        let serialized = serialize(&config).unwrap();
        let deserialized: HostConfig = parse_from_str(&serialized).unwrap();
        assert_eq!(config.system.port, deserialized.system.port);
    }

    #[test]
    fn test_validation_success() {
        let mut config = HostConfig::default();
        config.system.port = 8080;
        config.system.disk = "test".to_string();
        assert!(config.validate().is_ok());
        assert!(config.validate_for_install().is_ok());
    }

    #[test]
    fn test_validation_failure_port_zero() {
        let mut config = HostConfig::default();
        config.system.port = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_failure_empty_disk_install() {
        let mut config = HostConfig::default();
        config.system.port = 8080;
        config.system.disk = "".to_string();
        assert!(config.validate_for_install().is_err());
    }

    #[test]
    fn test_parse_from_str_invalid_toml() {
        let result = parse_from_str("invalid toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_default() {
        let default_str = serialize_default();
        let config: HostConfig = toml::from_str(&default_str).unwrap();
        assert!(config.validate().is_ok());
    }
}
