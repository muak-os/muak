use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

mod error;
pub use error::{ConfigError, Result};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../../../default.toml");

static CONFIG: OnceLock<HostConfig> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostConfig {
    pub system: SystemConfig,
    pub network: NetworkConfig,
    pub vm: VmConfig,
}

impl HostConfig {
    pub fn validate(&self) -> Result<()> {
        if self.system.port == 0 {
            return Err(ConfigError::ValidationError(
                "port must be greater than 0".to_string(),
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

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig::default(),
            network: NetworkConfig::default(),
            vm: VmConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
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

pub fn init() -> Result<()> {
    let config = load_from_path(Path::new(CONFIG_PATH))?;
    config.validate()?;
    CONFIG
        .set(config)
        .map_err(|_| ConfigError::AlreadyInitialized)
}

pub fn system() -> &'static SystemConfig {
    &config().system
}

pub fn network() -> &'static NetworkConfig {
    &config().network
}

pub fn vm() -> &'static VmConfig {
    &config().vm
}

pub fn config() -> &'static HostConfig {
    CONFIG.get().expect("Config not initialized")
}

pub fn try_config() -> Option<&'static HostConfig> {
    CONFIG.get()
}

pub fn default_config() -> HostConfig {
    HostConfig::default()
}

pub fn parse_from_str(contents: &str) -> Result<HostConfig> {
    let config: HostConfig = toml::from_str(contents)?;
    config.validate()?;
    Ok(config)
}

fn load_from_path(path: &Path) -> Result<HostConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(Into::into)
    } else {
        toml::from_str(DEFAULT_CONFIG).map_err(Into::into)
    }
}
