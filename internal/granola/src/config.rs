use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const CONFIG_PATH: &str = "/run/state/config.toml";
const DEFAULT_CONFIG: &str = include_str!("../../default.toml");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HostConfig {
    pub system: SystemConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    pub disk: String,
    pub image: String,
    pub extensions: Vec<String>,
    pub port: u16,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            disk: String::new(),
            image: "ghcr.io/sawangg/installer:latest".to_string(),
            extensions: Vec::new(),
            port: 50051,
        }
    }
}

impl HostConfig {
    pub fn load() -> Result<Self> {
        let path = Path::new(CONFIG_PATH);

        if path.exists() {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config from {}", CONFIG_PATH))?;
            let config: HostConfig = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config from {}", CONFIG_PATH))?;
            config.validate()?;
            Ok(config)
        } else {
            let config: HostConfig =
                toml::from_str(DEFAULT_CONFIG).context("Failed to parse default config")?;
            Ok(config)
        }
    }

    pub fn from_toml(contents: &str) -> Result<Self> {
        let config: HostConfig = toml::from_str(contents).context("Failed to parse config")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.system.port == 0 {
            bail!("port must be greater than 0");
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
