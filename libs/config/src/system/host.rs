use serde::{Deserialize, Serialize};

/// Host-level configuration for the Muak system.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostConfig {
    /// Hostname for this machine.
    pub name: String,
    /// System image reference (e.g. `ghcr.io/org/image:tag`).
    pub image: String,
    /// Additional system extension images.
    pub extensions: Vec<String>,
    /// Whether Secure Boot is enabled.
    pub secureboot: bool,
    /// gRPC port for the provision daemon.
    pub port: u16,
    /// NTP server address for time synchronization.
    pub ntp: String,
}
