//! Optional fleet roles; omission preserves standalone host behavior.

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

/// Explicitly selected role for the separately packaged fleet service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum Config {
    /// One durable management authority; no automatic failover.
    Manager {
        /// Maximum pending and approved node records, including revoked nodes.
        #[serde(default = "default_nodes")]
        max_nodes: usize,
        /// Time after which an observation is stale.
        #[serde(default = "default_stale")]
        stale_after_secs: u64,
    },
    /// Outbound inventory reporting to an explicitly trusted manager.
    Agent {
        /// HTTPS management endpoint, including port.
        manager: String,
        /// Persisted manager domain UUID, obtained using fleet info.
        domain: String,
        /// Public trust anchor obtained through a trusted administrator context.
        ca_pem: String,
        /// Expected TLS server identity.
        #[serde(default = "default_server_name")]
        server_name: String,
        /// Reporting and reconnect interval, in seconds.
        #[serde(default = "default_interval")]
        report_interval_secs: u64,
    },
}

impl Config {
    /// Validates explicit configuration without enabling an omitted role.
    ///
    /// # Errors
    ///
    /// Returns a validation error for invalid bounds or missing trust settings.
    pub fn validate(&self) -> Result<()> {
        let valid = match *self {
            Self::Manager {
                ref max_nodes,
                ref stale_after_secs,
            } => (1..=1024).contains(max_nodes) && (10..=86400).contains(stale_after_secs),
            Self::Agent {
                ref manager,
                ref domain,
                ref ca_pem,
                ref server_name,
                ref report_interval_secs,
            } => {
                manager.starts_with("https://")
                    && manager.len() <= 2048
                    && valid_domain(domain)
                    && ca_pem.contains("-----BEGIN CERTIFICATE-----")
                    && ca_pem.len() <= 0x4000
                    && !server_name.is_empty()
                    && server_name.len() <= 255
                    && (1..=300).contains(report_interval_secs)
            }
        };
        if !valid {
            return Err(ConfigError::ValidationError(
                "invalid fleet role, bounds or trust configuration".to_owned(),
            ));
        }
        Ok(())
    }
}

const fn default_nodes() -> usize {
    256
}
const fn default_stale() -> u64 {
    60
}
const fn default_interval() -> u64 {
    10
}
fn default_server_name() -> String {
    "muak-server".to_owned()
}

fn valid_domain(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|entry| {
            if [8, 13, 18, 23].contains(&entry.0) {
                entry.1 == b'-'
            } else {
                entry.1.is_ascii_digit() || (b'a'..=b'f').contains(&entry.1)
            }
        })
        && value.bytes().any(|byte| byte != b'0' && byte != b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_roundtrip_and_role_validation() {
        // ARRANGE
        let standalone = crate::system::Config::default();
        let encoded = crate::system::serialize(&standalone).unwrap();
        // ACT
        let decoded = crate::system::parse_from_str(&encoded).unwrap();
        let manager: Config = toml::from_str("role = 'manager'").unwrap();
        // ASSERT
        assert!(decoded.fleet.is_none());
        assert!(!encoded.contains("[fleet]"));
        assert!(standalone.same_as(&decoded));
        manager.validate().unwrap();
        let _error = toml::from_str::<Config>("role = 'manager'\nunknown = 1").unwrap_err();
        assert!(valid_domain("f0b96bb9-54ee-4a9f-abab-186e2178096a"));
        assert!(!valid_domain("00000000-0000-0000-0000-000000000000"));
        assert!(!valid_domain("f0b96bb9-54ee-4a9f-abab-186e2178096z"));
    }
}
