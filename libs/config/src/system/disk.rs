//! Disk selection configuration.

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

/// Disk selector matched either by device path or by matchers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
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

impl Selector {
    /// Creates a selector pinned to an explicit device path.
    #[must_use]
    pub fn device<S: Into<String>>(device: S) -> Self {
        Self {
            device: Some(device.into()),
            ..Self::default()
        }
    }

    /// Returns true when the selector constrains no disk at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.device.is_none() && !self.has_matcher() && self.rotational.is_none()
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

pub(super) fn validate_for_install(selector: &Selector) -> Result<()> {
    if let Some(device) = selector.device.as_deref() {
        if device.is_empty() {
            return Err(ConfigError::ValidationError(
                "disk device must not be empty".to_owned(),
            ));
        }
        if selector.has_matcher() {
            return Err(ConfigError::ValidationError(
                "disk cannot set both device and matchers".to_owned(),
            ));
        }

        return Ok(());
    }

    if !selector.has_matcher() {
        return Err(ConfigError::ValidationError(
            "disk requires a device or at least one matcher".to_owned(),
        ));
    }

    for (name, value) in selector.matchers() {
        if value == Some("") {
            return Err(ConfigError::ValidationError(format!(
                "disk.{name} matcher must not be empty"
            )));
        }
    }

    Ok(())
}

pub(super) fn validate_immutable(current: &Selector, installed: &Selector) -> Result<()> {
    if current != installed {
        return Err(ConfigError::ValidationError(format!(
            "disk cannot be changed after install (installed: {installed:?}, requested: {current:?})"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_constructor_pins_path() {
        // ARRANGE & ACT
        let selector = Selector::device("/dev/sda");

        // ASSERT
        assert_eq!(selector.device.as_deref(), Some("/dev/sda"));
        assert!(!selector.has_matcher());
    }

    #[test]
    fn is_empty_reports_unconstrained_selector() {
        // ARRANGE
        let empty = Selector::default();
        let pinned = Selector::device("/dev/sda");

        // ACT & ASSERT
        assert!(empty.is_empty());
        assert!(!pinned.is_empty());
    }

    #[test]
    fn validate_for_install_accepts_device() {
        // ARRANGE
        let selector = Selector::device("/dev/sda");

        // ACT
        let result = validate_for_install(&selector);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn validate_for_install_accepts_selector() {
        // ARRANGE
        let selector = Selector {
            serial: Some("ABC?".to_owned()),
            rotational: Some(false),
            ..Selector::default()
        };

        // ACT
        let result = validate_for_install(&selector);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn validate_for_install_rejects_empty_selector() {
        // ARRANGE
        let selector = Selector::default();

        // ACT
        let error = validate_for_install(&selector).unwrap_err();

        // ASSERT
        assert!(
            error
                .to_string()
                .contains("requires a device or at least one matcher"),
            "{error}"
        );
    }

    #[test]
    fn validate_for_install_rejects_device_and_matcher() {
        // ARRANGE
        let mut selector = Selector::device("/dev/sda");
        selector.serial = Some("ABC".to_owned());

        // ACT
        let error = validate_for_install(&selector).unwrap_err();

        // ASSERT
        assert!(
            error
                .to_string()
                .contains("cannot set both device and matchers"),
            "{error}"
        );
    }

    #[test]
    fn validate_for_install_rejects_empty_device() {
        // ARRANGE
        let selector = Selector::device("");

        // ACT
        let error = validate_for_install(&selector).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("disk device"), "{error}");
    }

    #[test]
    fn validate_for_install_rejects_empty_matcher() {
        // ARRANGE
        let selector = Selector {
            model: Some(String::new()),
            ..Selector::default()
        };

        // ACT
        let error = validate_for_install(&selector).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("disk.model matcher"), "{error}");
    }

    #[test]
    fn validate_immutable_rejects_changed_disk() {
        // ARRANGE
        let installed = Selector::device("/dev/sda");
        let current = Selector::device("/dev/sdb");

        // ACT
        let error = validate_immutable(&current, &installed).unwrap_err();

        // ASSERT
        assert!(
            error.to_string().contains("disk cannot be changed"),
            "{error}"
        );
    }

    #[test]
    fn validate_immutable_accepts_same_disk() {
        // ARRANGE
        let installed = Selector::device("/dev/sda");
        let current = Selector::device("/dev/sda");

        // ACT
        let result = validate_immutable(&current, &installed);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn rejects_the_removed_role_field() {
        // ARRANGE
        let encoded = "role = \"system\"\ndevice = \"/dev/sda\"";

        // ACT
        let result = toml::from_str::<Selector>(encoded);

        // ASSERT
        assert!(result.is_err(), "role is no longer a known field");
    }
}
