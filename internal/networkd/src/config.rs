use std::path::Path;

use serde::Deserialize;

const CONFIG_PATH: &str = "/run/state/config.toml";

pub const RESOLV_CONF_PATH: &str = "/run/resolv.conf";
pub const BRIDGE_CREATE_RETRIES: u8 = 30;
pub const BRIDGE_CREATE_RETRY_DELAY_MS: u64 = 100;
pub const INTERFACE_ENSLAVE_RETRIES: u8 = 5;
pub const INTERFACE_ENSLAVE_RETRY_DELAY_MS: u64 = 100;

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub bridge: String,
    pub carrier_timeout: u64,
    pub check_interval_secs: u64,
    pub probe_timeout_secs: u64,
    pub overall_timeout_secs: u64,
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
    connectivity: Option<ConnectivitySection>,
    ipv6: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ConnectivitySection {
    check_interval_secs: Option<u64>,
    probe_timeout_secs: Option<u64>,
    overall_timeout_secs: Option<u64>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bridge: "br0".to_string(),
            carrier_timeout: 6,
            check_interval_secs: 60,
            probe_timeout_secs: 5,
            overall_timeout_secs: 15,
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
            connectivity: None,
            ipv6: None,
        });
        let connectivity = network.connectivity.unwrap_or(ConnectivitySection {
            check_interval_secs: None,
            probe_timeout_secs: None,
            overall_timeout_secs: None,
        });

        Self {
            bridge: network.bridge.unwrap_or(defaults.bridge),
            carrier_timeout: network.carrier_timeout.unwrap_or(defaults.carrier_timeout),
            check_interval_secs: connectivity
                .check_interval_secs
                .unwrap_or(defaults.check_interval_secs),
            probe_timeout_secs: connectivity
                .probe_timeout_secs
                .unwrap_or(defaults.probe_timeout_secs),
            overall_timeout_secs: connectivity
                .overall_timeout_secs
                .unwrap_or(defaults.overall_timeout_secs),
            ipv6: network.ipv6.unwrap_or(defaults.ipv6),
        }
    }
}
