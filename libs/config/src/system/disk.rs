//! Disk assignment configuration.

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

/// Role a disk plays in the partition layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Disk hosting the system image.
    #[default]
    System,
}

/// Disk assignment entry selected either by device path or by matchers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Role of this disk.
    pub role: Kind,
    /// Explicit device path (e.g. `/dev/sda`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Serial number matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// World wide identifier matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wwid: Option<String>,
    /// Model name matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Kernel subsystem matcher (e.g. `nvme`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    /// Physical bus path matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus_path: Option<String>,
    /// Restrict selection to rotational or non-rotational disks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotational: Option<bool>,
}

impl Entry {
    /// Creates an entry pinned to an explicit device path.
    #[must_use]
    pub fn device<S: Into<String>>(role: Kind, device: S) -> Self {
        Self {
            role,
            device: Some(device.into()),
            ..Self::default()
        }
    }

    /// Creates an entry that selects a disk by matchers.
    #[must_use]
    pub fn selector(role: Kind) -> Self {
        Self {
            role,
            ..Self::default()
        }
    }

    /// Returns true when the entry selects disks by matchers instead of a device path.
    #[must_use]
    pub fn is_selector(&self) -> bool {
        self.device.is_none() && self.has_matcher()
    }

    fn matchers(&self) -> [(&'static str, Option<&str>); 5] {
        [
            ("serial", self.serial.as_deref()),
            ("wwid", self.wwid.as_deref()),
            ("model", self.model.as_deref()),
            ("subsystem", self.subsystem.as_deref()),
            ("bus_path", self.bus_path.as_deref()),
        ]
    }

    fn has_matcher(&self) -> bool {
        self.matchers()
            .into_iter()
            .any(|(_, value)| value.is_some())
    }
}

/// Returns the configured system disk entry, if any.
#[must_use]
pub fn system(entries: &[Entry]) -> Option<&Entry> {
    entries.iter().find(|entry| entry.role == Kind::System)
}

pub(super) fn validate_for_install(entries: &[Entry]) -> Result<()> {
    let systems = entries
        .iter()
        .filter(|entry| entry.role == Kind::System)
        .count();
    match systems {
        1 => {}
        0 => {
            return Err(ConfigError::ValidationError(
                "disks must contain exactly one system entry".to_owned(),
            ));
        }
        _ => {
            return Err(ConfigError::ValidationError(
                "disks must contain exactly one system entry, found several".to_owned(),
            ));
        }
    }

    for entry in entries {
        validate_entry(entry)?;
    }

    Ok(())
}

pub(super) fn validate_immutable(current: &[Entry], installed: &[Entry]) -> Result<()> {
    let before = system(installed);
    let after = system(current);
    if before != after {
        return Err(ConfigError::ValidationError(format!(
            "disk.system cannot be changed after install (installed: {before:?}, requested: {after:?})"
        )));
    }

    Ok(())
}

fn validate_entry(entry: &Entry) -> Result<()> {
    if let Some(device) = entry.device.as_deref() {
        if device.is_empty() {
            return Err(ConfigError::ValidationError(
                "disk.system device must not be empty".to_owned(),
            ));
        }
        if entry.has_matcher() {
            return Err(ConfigError::ValidationError(
                "disk.system entry cannot set both device and matchers".to_owned(),
            ));
        }

        return Ok(());
    }

    if !entry.has_matcher() {
        return Err(ConfigError::ValidationError(
            "disk.system entry requires a device or at least one matcher".to_owned(),
        ));
    }

    for (name, value) in entry.matchers() {
        if value == Some("") {
            return Err(ConfigError::ValidationError(format!(
                "disk.system.{name} matcher must not be empty"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_device(path: &str) -> Entry {
        Entry::device(Kind::System, path)
    }

    #[test]
    fn device_constructor_pins_path() {
        // ARRANGE & ACT
        let entry = system_device("/dev/sda");

        // ASSERT
        assert_eq!(entry.role, Kind::System);
        assert_eq!(entry.device.as_deref(), Some("/dev/sda"));
        assert!(!entry.is_selector());
    }

    #[test]
    fn selector_constructor_is_selector() {
        // ARRANGE
        let mut entry = Entry::selector(Kind::System);
        entry.serial = Some("ABC*".to_owned());

        // ACT & ASSERT
        assert!(entry.is_selector());
        assert_eq!(entry.device, None);
    }

    #[test]
    fn system_lookup_returns_the_system_entry() {
        // ARRANGE
        let entries = vec![system_device("/dev/sda")];

        // ACT
        let found = system(&entries);

        // ASSERT
        assert_eq!(
            found.and_then(|entry| entry.device.as_deref()),
            Some("/dev/sda")
        );
    }

    #[test]
    fn validate_for_install_accepts_system_only() {
        // ARRANGE
        let entries = vec![system_device("/dev/sda")];

        // ACT
        let result = validate_for_install(&entries);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn validate_for_install_accepts_selector() {
        // ARRANGE
        let mut entry = Entry::selector(Kind::System);
        entry.serial = Some("ABC?".to_owned());
        entry.rotational = Some(false);

        // ACT
        let result = validate_for_install(&[entry]);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn validate_for_install_rejects_missing_system() {
        // ARRANGE
        let entries: Vec<Entry> = Vec::new();

        // ACT
        let result = validate_for_install(&entries);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_for_install_rejects_multiple_system() {
        // ARRANGE
        let entries = vec![system_device("/dev/sda"), system_device("/dev/sdb")];

        // ACT
        let result = validate_for_install(&entries);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_for_install_rejects_device_and_matcher() {
        // ARRANGE
        let mut entry = system_device("/dev/sda");
        entry.serial = Some("ABC".to_owned());

        // ACT
        let result = validate_for_install(&[entry]);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_for_install_rejects_empty_entry() {
        // ARRANGE
        let entries = vec![Entry::selector(Kind::System)];

        // ACT
        let result = validate_for_install(&entries);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_for_install_rejects_empty_device() {
        // ARRANGE
        let entries = vec![system_device("")];

        // ACT
        let result = validate_for_install(&entries);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_for_install_rejects_empty_matcher() {
        // ARRANGE
        let mut entry = Entry::selector(Kind::System);
        entry.model = Some(String::new());

        // ACT
        let result = validate_for_install(&[entry]);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_immutable_reports_changed_role() {
        // ARRANGE
        let installed = vec![system_device("/dev/sda")];
        let current = vec![system_device("/dev/sdb")];

        // ACT
        let error = validate_immutable(&current, &installed).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("disk.system"), "{error}");
    }

    #[test]
    fn entry_requires_role() {
        // ARRANGE
        let missing = "serial = \"ABC\"";
        let present = "role = \"system\"\nserial = \"ABC\"";

        // ACT
        let missing = toml::from_str::<Entry>(missing);
        let present = toml::from_str::<Entry>(present);

        // ASSERT
        assert!(missing.is_err(), "role must be required");
        assert_eq!(present.unwrap().role, Kind::System);
    }

    #[test]
    fn rejects_the_removed_data_role() {
        // ARRANGE
        let encoded = "role = \"data\"\nserial = \"ABC\"";

        // ACT
        let result = toml::from_str::<Entry>(encoded);

        // ASSERT
        assert!(result.is_err(), "the data role no longer exists");
    }

    #[test]
    fn kind_serializes_lowercase() {
        // ARRANGE
        let entry = system_device("/dev/sda");

        // ACT
        let encoded = toml::to_string(&entry).unwrap();

        // ASSERT
        assert!(encoded.contains("role = \"system\""), "{encoded}");
    }
}
