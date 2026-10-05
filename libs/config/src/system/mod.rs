//! Immutable host system configuration.

pub mod disk;
pub mod host;
pub mod network;
pub mod vm;

use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::codec::{Codec as _, TomlCodec};
use crate::error::{ConfigError, Result};

/// Path to the system config file on disk.
pub const CONFIG_PATH: &str = "/run/state/config.toml";
/// File extension for system config files.
pub const CONFIG_EXTENSION: &str = "toml";

/// Schema version of the system config document.
pub const API_VERSION: &str = "muak.dev/config/v1-beta";

pub(crate) static CONFIG: OnceLock<Config> = OnceLock::new();

const DEFAULT_CONFIG: &str = include_str!("../../default.toml");

/// Top-level system configuration covering host, disk, network, and VM settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version of this document.
    pub api_version: String,
    /// Host-level configuration (name, registry, channel, version, ports, etc.).
    #[serde(default)]
    pub host: host::Config,
    /// Disk partition layout configuration.
    #[serde(default)]
    pub disk: disk::Config,
    /// Network configuration (interfaces, DNS, IPv6).
    #[serde(default)]
    pub network: network::Config,
    /// Virtual machine configuration.
    #[serde(default)]
    pub vm: vm::Config,
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG).unwrap_or_else(|_| Config {
            api_version: API_VERSION.to_owned(),
            host: host::Config::default(),
            disk: disk::Config::default(),
            network: network::Config::default(),
            vm: vm::Config::default(),
        })
    }
}

impl Config {
    /// Validates that required fields are present and sensible.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::ValidationError`] when a required field is
    /// missing or has an unsupported value.
    pub fn validate(&self) -> Result<()> {
        if self.host.name.is_empty() {
            return Err(ConfigError::ValidationError(
                "host.name must be specified".to_owned(),
            ));
        }
        if self.host.port == 0 {
            return Err(ConfigError::ValidationError(
                "host.port must be greater than 0".to_owned(),
            ));
        }
        if !matches!(self.host.clock.as_str(), "" | "auto" | "hypervisor" | "ntp") {
            return Err(ConfigError::ValidationError(
                "host.clock must be one of \"auto\", \"hypervisor\" or \"ntp\"".to_owned(),
            ));
        }
        if self.host.ntp.is_empty() && self.host.clock != "hypervisor" {
            return Err(ConfigError::ValidationError(
                "host.ntp must be specified".to_owned(),
            ));
        }

        Ok(())
    }

    /// Validates that the config is complete enough for installation.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::ValidationError`] when required installation fields are missing.
    pub fn validate_for_install(&self) -> Result<()> {
        self.validate()?;
        if self.host.version.is_empty() {
            return Err(ConfigError::ValidationError(
                "host.version must be set".to_owned(),
            ));
        }
        self.disk.validate_for_install()
    }

    /// Validates that the config is acceptable for an update operation.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::ValidationError`] when the requested config
    /// changes immutable fields or the base validation fails.
    pub fn validate_for_update(&self, installed: &Config) -> Result<()> {
        self.validate()?;
        self.disk.validate_immutable(&installed.disk)?;
        if installed.host.secureboot && !self.host.secureboot {
            return Err(ConfigError::ValidationError(
                "host.secureboot cannot be disabled after Secure Boot keys have been enrolled"
                    .to_owned(),
            ));
        }

        Ok(())
    }
}

/// Returns the global system configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NotInitialized`] when [`init()`] has not been
/// called.
pub fn config() -> Result<&'static Config> {
    try_config().ok_or(ConfigError::NotInitialized)
}

/// Returns the global host configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NotInitialized`] when [`init()`] has not been called.
pub fn host() -> Result<&'static host::Config> {
    config().map(|system| &system.host)
}

/// Returns the global network configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NotInitialized`] when [`init()`] has not been called.
pub fn network() -> Result<&'static network::Config> {
    config().map(|system| &system.network)
}

/// Returns the global VM configuration.
///
/// # Errors
///
/// Returns [`ConfigError::NotInitialized`] when [`init()`] has not been called.
pub fn vm() -> Result<&'static vm::Config> {
    config().map(|system| &system.vm)
}

/// Returns the global system configuration, or `None` before [`init()`].
#[must_use]
pub fn try_config() -> Option<&'static Config> {
    CONFIG.get()
}

/// Returns `true` when moving from `previous` to `next` could cut the
/// operator's access to the machine.
#[must_use]
pub fn isolates(previous: &Config, next: &Config) -> bool {
    previous.network != next.network || previous.host.port != next.host.port
}

/// Initializes the host config.
///
/// # Errors
///
/// Returns an error when loading or validating the config from
/// [`CONFIG_PATH`] fails, or when the config was already initialized.
pub fn init() -> Result<()> {
    let config = load_from_path(Path::new(CONFIG_PATH))?;
    config.validate()?;
    CONFIG
        .set(config)
        .map_err(|_existing| ConfigError::AlreadyInitialized)?;

    Ok(())
}

/// Serializes a [`Config`] to a string.
///
/// # Errors
///
/// Returns an error when the config cannot be encoded to TOML.
pub fn serialize(config: &Config) -> Result<String> {
    TomlCodec::encode(config)
}

/// Serializes the default system configuration to a string.
#[must_use]
pub fn serialize_default() -> String {
    TomlCodec::encode(&Config::default()).unwrap_or_default()
}

/// Parses a [`Config`] from a string, validating it.
///
/// # Errors
///
/// Returns an error when parsing fails, the schema version is unsupported, or validation fails.
pub fn parse_from_str(contents: &str) -> Result<Config> {
    let config = decode(contents)?;
    config.validate()?;
    Ok(config)
}

/// Deserializes a [`Config`], rejecting unsupported schema versions.
fn decode(contents: &str) -> Result<Config> {
    let config: Config = TomlCodec::decode(contents)?;
    if config.api_version != API_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            found: config.api_version,
            supported: API_VERSION.to_owned(),
        });
    }

    Ok(config)
}

/// Diffs two config strings, returning `(field_path, before, after)` for each changed field.
///
/// # Errors
///
/// Returns an error when either document cannot be decoded or uses an unsupported schema version.
pub fn diff(before: &str, after: &str) -> Result<Vec<(String, String, String)>> {
    let before = toml::Value::try_from(decode(before)?)?;
    let after = toml::Value::try_from(decode(after)?)?;
    let mut changes = Vec::new();
    diff_values(&mut changes, "", &before, &after);

    Ok(changes)
}

/// Loads system config from a file, falling back to defaults if not found.
///
/// # Errors
///
/// Returns an error when the file exists but cannot be read or decoded.
pub fn load_from_path(path: &Path) -> Result<Config> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        TomlCodec::decode(&contents)
    } else {
        decode(DEFAULT_CONFIG)
    }
}

/// Atomically writes `contents` to `path` via a sibling temporary file and
/// rename, so readers never observe a partially written file.
///
/// # Errors
///
/// Returns [`ConfigError`] when writing or renaming fails.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;

    Ok(())
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn diff_values(
    changes: &mut Vec<(String, String, String)>,
    prefix: &str,
    before: &toml::Value,
    after: &toml::Value,
) {
    match (before.as_table(), after.as_table()) {
        (Some(before_table), Some(after_table)) => {
            for (key, before_value) in before_table {
                let path = join_path(prefix, key.as_str());
                match after_table.get(key.as_str()) {
                    Some(after_value) => diff_values(changes, &path, before_value, after_value),
                    None => changes.push((path, before_value.to_string(), String::new())),
                }
            }
            for (key, after_value) in after_table
                .iter()
                .filter(|entry| !before_table.contains_key(entry.0.as_str()))
            {
                changes.push((
                    join_path(prefix, key),
                    String::new(),
                    after_value.to_string(),
                ));
            }
        }
        _ => {
            if before != after {
                changes.push((prefix.to_owned(), before.to_string(), after.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_version(body: &str) -> String {
        format!("api_version = \"{API_VERSION}\"\n{body}")
    }

    #[test]
    fn host_config_serialization() {
        // ARRANGE
        let config = Config::default();

        // ACT
        let serialized = TomlCodec::encode(&config).unwrap();
        let deserialized: Config = TomlCodec::decode(&serialized).unwrap();

        // ASSERT
        assert_eq!(config.host.port, deserialized.host.port);
    }

    #[test]
    fn validation_success() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 8080;
        config.host.version = "v1.0.0".to_owned();
        config.disk.system = "/dev/sda".to_owned();

        // ACT & ASSERT
        config.validate().unwrap();
        config.validate_for_install().unwrap();
    }

    #[test]
    fn validation_failure_empty_version_install() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 8080;
        config.disk.system = "/dev/sda".to_owned();

        // ACT & ASSERT
        assert!(config.validate_for_install().is_err());
    }

    #[test]
    fn validation_failure_port_zero() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 0;

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_failure_empty_disk_install() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 8080;
        config.host.version = "v1.0.0".to_owned();
        config.disk.system = String::new();

        // ACT & ASSERT
        assert!(config.validate_for_install().is_err());
    }

    #[test]
    fn parse_from_str_invalid_toml() {
        // ACT & ASSERT
        let result = TomlCodec::decode::<Config>("invalid toml");
        result.unwrap_err();
    }

    #[test]
    fn serialize_default() {
        // ACT
        let default_str = TomlCodec::encode(&Config::default()).unwrap();
        let config: Config = TomlCodec::decode(&default_str).unwrap();

        // ASSERT
        config.validate().unwrap();
    }

    #[test]
    fn validate_for_update_rejects_system_disk_change() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.disk.system = "/dev/sda".to_owned();

        let mut requested = installed.clone();
        requested.disk.system = "/dev/sdb".to_owned();

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn validate_for_update_rejects_data_disk_change() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.disk.system = "/dev/sda".to_owned();
        installed.disk.data = Some("/dev/sdb".to_owned());

        let mut requested = installed.clone();
        requested.disk.data = Some("/dev/sdc".to_owned());

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn validate_for_update_allows_secureboot_false_to_true() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.host.secureboot = false;

        let mut requested = installed.clone();
        requested.host.secureboot = true;

        // ACT & ASSERT
        requested.validate_for_update(&installed).unwrap();
    }

    #[test]
    fn validate_for_update_rejects_secureboot_true_to_false() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.host.secureboot = true;

        let mut requested = installed.clone();
        requested.host.secureboot = false;

        // ACT & ASSERT
        assert!(requested.validate_for_update(&installed).is_err());
    }

    #[test]
    fn validate_for_update_allows_secureboot_unchanged_false() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.host.secureboot = false;

        // ACT & ASSERT
        installed.clone().validate_for_update(&installed).unwrap();
    }

    #[test]
    fn validate_for_update_allows_secureboot_unchanged_true() {
        // ARRANGE
        let mut installed = Config::default();
        installed.host.port = 8080;
        installed.host.secureboot = true;

        // ACT & ASSERT
        installed.clone().validate_for_update(&installed).unwrap();
    }

    #[test]
    fn config_rejects_unknown_sections() {
        // ARRANGE
        let toml_str = with_version(
            r#"
[host]
name = "muak"
port = 50051

[system]
image = "10.0.2.2:5000/installer:latest"
"#,
        );

        // ACT
        let result = parse_from_str(&toml_str);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn validation_failure_empty_ntp() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 8080;
        config.host.ntp = String::new();

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_accepts_hypervisor_clock_without_ntp() {
        // ARRANGE
        let mut config = Config::default();
        config.host.clock = "hypervisor".to_owned();
        config.host.ntp = String::new();

        // ACT
        let result = config.validate();

        // ASSERT
        assert!(
            result.is_ok(),
            "hypervisor clock must not require host.ntp: {result:?}"
        );
    }

    #[test]
    fn validation_failure_unknown_clock() {
        // ARRANGE
        let mut config = Config::default();
        config.host.clock = "ptp".to_owned();

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn validation_failure_empty_name() {
        // ARRANGE
        let mut config = Config::default();
        config.host.name = String::new();
        config.host.port = 8080;

        // ACT & ASSERT
        assert!(config.validate().is_err());
    }

    #[test]
    fn serialize_round_trip() {
        // ARRANGE
        let mut config = Config::default();
        config.host.port = 9090;
        config.host.name = "testhost".to_owned();
        config.disk.system = "/dev/nvme0n1".to_owned();
        config.network.ipv6 = true;
        config.network.dns = vec!["9.9.9.9".parse().unwrap()];
        config.vm.auto_restart = true;

        // ACT
        let serialized = serialize(&config).unwrap();
        let restored: Config = TomlCodec::decode(&serialized).unwrap();

        // ASSERT
        assert_eq!(restored.api_version, API_VERSION);
        assert_eq!(restored.host.port, 9090);
        assert_eq!(restored.host.name, "testhost");
        assert_eq!(restored.disk.system, "/dev/nvme0n1");
        assert!(restored.network.ipv6);
        assert_eq!(
            restored.network.dns,
            vec!["9.9.9.9".parse::<std::net::IpAddr>().unwrap()]
        );
        assert!(restored.vm.auto_restart);
    }

    #[test]
    fn parse_from_str_valid() {
        // ARRANGE
        let toml_str = with_version(
            r#"
[host]
name = "myhost"
port = 1234
ntp = "pool.ntp.org"
"#,
        );

        // ACT
        let config = parse_from_str(&toml_str).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "myhost");
        assert_eq!(config.host.port, 1234);
    }

    #[test]
    fn parse_from_str_invalid_format_error() {
        // ACT
        let result = parse_from_str("[[[ invalid");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn parse_from_str_validation_error() {
        // ARRANGE
        let invalid = with_version("[host]\nport = 0\n");

        // ACT
        let result = parse_from_str(&invalid);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn load_from_path_existing_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content =
            with_version("[host]\nname = \"loaded\"\nport = 7777\nntp = \"pool.ntp.org\"\n");
        std::fs::write(&path, content).unwrap();

        // ACT
        let config = load_from_path(&path).unwrap();

        // ASSERT
        assert_eq!(config.host.name, "loaded");
        assert_eq!(config.host.port, 7777);
    }

    #[test]
    fn load_from_path_nonexistent_uses_default() {
        // ARRANGE
        let path = std::path::Path::new("/nonexistent/config.toml");

        // ACT
        let config = load_from_path(path).unwrap();

        // ASSERT
        config.validate().unwrap();
    }

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_temporary() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "old = true").unwrap();

        // ACT
        write_atomic(&path, b"new = 1").unwrap();

        // ASSERT
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "new = 1");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary file may remain");
    }

    #[test]
    fn diff_no_changes() {
        // ARRANGE
        let config = with_version("[host]\nname = \"x\"\nport = 1\n");

        // ACT
        let changes = diff(&config, &config).unwrap();

        // ASSERT
        assert_eq!(
            changes,
            Vec::<(
                alloc::string::String,
                alloc::string::String,
                alloc::string::String
            )>::new()
        );
    }

    #[test]
    fn diff_changed_scalar() {
        // ARRANGE
        let before = with_version("[host]\nname = \"alpha\"\nport = 8080\n");
        let after = with_version("[host]\nname = \"beta\"\nport = 8080\n");

        // ACT
        let changes = diff(&before, &after).unwrap();

        // ASSERT
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes.first().map(|change| change.0.as_str()),
            Some("host.name")
        );
        assert!(
            changes
                .first()
                .is_some_and(|change| change.1.contains("alpha"))
        );
        assert!(
            changes
                .first()
                .is_some_and(|change| change.2.contains("beta"))
        );
    }

    #[test]
    fn diff_added_key() {
        // ARRANGE
        let before = with_version("[host]\nport = 1\n\n[network]\nipv6 = false\n");
        let after = with_version("[host]\nport = 1\n\n[network]\nipv6 = true\n");

        // ACT
        let changes = diff(&before, &after).unwrap();

        // ASSERT
        let found = changes.iter().any(|change| {
            change.0 == "network.ipv6" && change.1.contains("false") && change.2.contains("true")
        });
        assert!(found, "expected network.ipv6 change, got: {changes:?}");
    }

    #[test]
    fn diff_removed_key() {
        // ARRANGE
        let before = with_version("[host]\nport = 1\n\n[network]\nipv6 = true\n");
        let after = with_version("[host]\nport = 1\n\n[network]\nipv6 = false\n");

        // ACT
        let changes = diff(&before, &after).unwrap();

        // ASSERT
        let found = changes.iter().any(|change| {
            change.0 == "network.ipv6" && change.1.contains("true") && change.2.contains("false")
        });
        assert!(found, "expected network.ipv6 change, got: {changes:?}");
    }

    #[test]
    fn diff_section_added_shows_field_changes_against_defaults() {
        // ARRANGE
        let before = with_version("[host]\nport = 1\n");
        let after = with_version("[host]\nport = 1\n\n[network]\nipv6 = true\n");

        // ACT
        let changes = diff(&before, &after).unwrap();

        // ASSERT
        assert_eq!(
            changes,
            vec![(
                "network.ipv6".to_owned(),
                "false".to_owned(),
                "true".to_owned()
            )],
            "added sections must diff field-by-field against defaults"
        );
    }

    #[test]
    fn diff_section_removed_shows_field_changes_against_defaults() {
        // ARRANGE
        let before = with_version("[host]\nport = 1\n\n[network]\nipv6 = true\n");
        let after = with_version("[host]\nport = 1\n");

        // ACT
        let changes = diff(&before, &after).unwrap();

        // ASSERT
        assert_eq!(
            changes,
            vec![(
                "network.ipv6".to_owned(),
                "true".to_owned(),
                "false".to_owned()
            )],
            "removed sections must diff field-by-field against defaults"
        );
    }

    #[test]
    fn default_matches_default_config() {
        let from: Config = TomlCodec::decode(DEFAULT_CONFIG).unwrap();
        let from_default = Config::default();

        let encoded = TomlCodec::encode(&from).unwrap();
        let default_str = TomlCodec::encode(&from_default).unwrap();
        assert_eq!(
            encoded, default_str,
            "Default impl has drifted from default config"
        );
    }

    #[test]
    fn host_config_fields() {
        // ARRANGE
        let mut config = Config::default();
        config.host.version = "v1.0.0".to_owned();
        config.host.extensions = vec!["ext1".to_owned()];
        config.host.ntp = "pool.ntp.org".to_owned();
        config.host.secureboot = true;

        // ACT
        let serialized = serialize(&config).unwrap();
        let restored: Config = TomlCodec::decode(&serialized).unwrap();

        // ASSERT
        assert_eq!(restored.host.version, "v1.0.0");
        assert_eq!(restored.host.extensions, vec!["ext1"]);
        assert_eq!(restored.host.ntp, "pool.ntp.org");
        assert!(restored.host.secureboot);
    }

    #[test]
    fn rejects_unsupported_api_version() {
        // ARRANGE
        let toml = "api_version = \"muak.dev/config/v999\"\n[host]\nname = \"x\"\nport = 1\n";

        // ACT
        let result = parse_from_str(toml);

        // ASSERT
        assert!(matches!(
            result,
            Err(ConfigError::UnsupportedVersion { found, supported })
            if found == "muak.dev/config/v999" && supported == API_VERSION
        ));
    }

    #[test]
    fn rejects_missing_api_version() {
        // ARRANGE
        let toml = "[host]\nname = \"x\"\nport = 1\n";

        // ACT
        let result = parse_from_str(toml);

        // ASSERT
        let error = result.expect_err("missing api_version must be rejected");
        assert!(error.to_string().contains("api_version"), "{error}");
    }

    #[test]
    fn network_change_isolates() {
        // ARRANGE
        let previous = Config::default();
        let mut next = previous.clone();
        next.network.dns = vec!["1.1.1.1".parse().expect("valid ip")];

        // ACT
        let isolates = isolates(&previous, &next);

        // ASSERT
        assert!(isolates, "network changes must require confirmation");
    }

    #[test]
    fn api_port_change_isolates() {
        // ARRANGE
        let previous = Config::default();
        let mut next = previous.clone();
        next.host.port = 8080;

        // ACT
        let isolates = isolates(&previous, &next);

        // ASSERT
        assert!(isolates, "API port changes must require confirmation");
    }

    #[test]
    fn other_changes_do_not_isolate() {
        // ARRANGE
        let previous = Config::default();
        let mut next = previous.clone();
        next.host.version = "v2.0.0".to_owned();
        next.host.secureboot = true;
        next.vm.auto_restart = false;

        // ACT
        let isolates = isolates(&previous, &next);

        // ASSERT
        assert!(
            !isolates,
            "non-reachability changes must not require confirmation"
        );
    }

    #[test]
    fn unchanged_configs_do_not_isolate() {
        // ARRANGE
        let previous = Config::default();
        let next = Config::default();

        // ACT & ASSERT
        assert!(!isolates(&previous, &next));
    }
}
