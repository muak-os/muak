//! Host-level settings (name, registry, version, ports, etc.).

use serde::{Deserialize, Serialize};

/// Host-level configuration for the Muak system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Hostname for this machine.
    pub name: String,
    /// Registry serving the catalog (e.g. `ghcr.io/muak-os`).
    pub registry: String,
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

impl Config {
    /// Returns `true` when both host configs are semantically equivalent.
    ///
    /// Extension ordering is ignored, so reordering them is not a change.
    #[must_use]
    pub fn same_as(&self, other: &Config) -> bool {
        let mut normalized = self.clone();
        let mut other_normalized = other.clone();
        normalized.extensions.sort();
        other_normalized.extensions.sort();

        normalized == other_normalized
    }
}
