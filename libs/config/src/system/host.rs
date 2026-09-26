use serde::{Deserialize, Serialize};

/// Host-level configuration for the Muak system.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostConfig {
    /// Hostname for this machine.
    pub name: String,
    /// Registry serving the catalog (e.g. `ghcr.io/muak-os`).
    pub registry: String,
    /// Catalog channel this machine tracks (e.g. `stable`).
    #[serde(default = "default_channel")]
    pub channel: String,
    /// Deployed catalog release (e.g. `v1.1.0`).
    pub version: String,
    /// Additional system extension images.
    pub extensions: Vec<String>,
    /// Whether Secure Boot is enabled.
    pub secureboot: bool,
    /// gRPC port for the provision daemon.
    pub port: u16,
    /// Which clock to set the system time from.
    pub clock: String,
    /// NTP server address for time synchronization.
    pub ntp: String,
}

fn default_channel() -> String {
    "stable".to_owned()
}
