use std::path::Path;

use serde::Deserialize;

const CONFIG_PATH: &str = "/run/state/config.toml";

pub const RESOLV_CONF_PATH: &str = "/run/resolv.conf";
pub const BRIDGE_CREATE_RETRIES: u8 = 30;
pub const BRIDGE_CREATE_RETRY_DELAY_MS: u64 = 100;
pub const INTERFACE_ENSLAVE_RETRIES: u8 = 5;
pub const INTERFACE_ENSLAVE_RETRY_DELAY_MS: u64 = 100;

pub const CONNECTIVITY_CHECK_INTERVAL_SECS: u64 = 60;
pub const CONNECTIVITY_PROBE_TIMEOUT_SECS: u64 = 5;
pub const CONNECTIVITY_OVERALL_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub bridge: String,
    pub carrier_timeout: u64,
    pub ipv6: bool,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    network: Option<NetworkSection>,
}

#[derive(Debug, Deserialize)]
struct NetworkSection {
    bridge: Option<String>,
    carrier_timeout: Option<u64>,
    ipv6: Option<bool>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bridge: "br0".to_string(),
            carrier_timeout: 6,
            ipv6: true,
        }
    }
}

impl NetworkConfig {
    pub fn load() -> Self {
        let path = Path::new(CONFIG_PATH);
        if !path.exists() {
            return Self::default();
        }

        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                kmsg::warn!(@ "networkd", "Failed to read config: {}, using defaults", e);
                return Self::default();
            }
        };

        let config: ConfigFile = match toml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                kmsg::warn!(@ "networkd", "Failed to parse config: {}, using defaults", e);
                return Self::default();
            }
        };

        let defaults = Self::default();
        let network = config.network.unwrap_or(NetworkSection {
            bridge: None,
            carrier_timeout: None,
            ipv6: None,
        });

        Self {
            bridge: network.bridge.unwrap_or(defaults.bridge),
            carrier_timeout: network.carrier_timeout.unwrap_or(defaults.carrier_timeout),
            ipv6: network.ipv6.unwrap_or(defaults.ipv6),
        }
    }
}
