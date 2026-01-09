use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../../default.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MuakConfig {
    pub system: SystemConfig,
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    pub disk: String,
    pub image: String,
    pub extensions: Vec<String>,
    pub api_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub bridge: String,
    pub connectivity: ConnectivityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectivityConfig {
    pub check_interval_secs: u64,
    pub probe_timeout_secs: u64,
    pub overall_timeout_secs: u64,
}

impl Default for MuakConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig::default(),
            network: NetworkConfig::default(),
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            disk: String::new(),
            image: "ghcr.io/sawangg/installer:latest".to_string(),
            extensions: vec![],
            api_port: 50051,
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bridge: "br0".to_string(),
            connectivity: ConnectivityConfig::default(),
        }
    }
}

impl Default for ConnectivityConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 60,
            probe_timeout_secs: 5,
            overall_timeout_secs: 15,
        }
    }
}

impl MuakConfig {
    pub fn load() -> Result<Self> {
        let path = Path::new(CONFIG_PATH);

        if path.exists() {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config from {}", CONFIG_PATH))?;
            let config: MuakConfig = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config from {}", CONFIG_PATH))?;
            config.validate()?;
            Ok(config)
        } else {
            let config: MuakConfig =
                toml::from_str(DEFAULT_CONFIG).context("Failed to parse default config")?;
            Ok(config)
        }
    }

    pub fn from_toml(contents: &str) -> Result<Self> {
        let config: MuakConfig = toml::from_str(contents).context("Failed to parse config")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.system.api_port == 0 {
            bail!("api_port must be greater than 0");
        }
        if self.network.bridge.is_empty() {
            bail!("bridge name cannot be empty");
        }
        Ok(())
    }

    pub fn validate_for_install(&self) -> Result<()> {
        self.validate()?;
        if self.system.disk.is_empty() {
            bail!("system.disk must be specified for installation");
        }
        Ok(())
    }
}
