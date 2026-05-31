use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

/// Disk assignment configuration for system and data partitions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DiskConfig {
    pub system: String,
    pub data: Option<String>,
}

impl DiskConfig {
    /// Returns the effective data disk path, falling back to `system` when `data` is unset.
    pub fn data_disk(&self) -> &str {
        match &self.data {
            Some(d) if !d.is_empty() => d.as_str(),
            _ => &self.system,
        }
    }

    /// Returns true when the data partition lives on a separate physical disk.
    pub fn is_split(&self) -> bool {
        matches!(&self.data, Some(d) if !d.is_empty() && d != &self.system)
    }

    pub(super) fn validate_for_install(&self) -> Result<()> {
        if self.system.is_empty() {
            return Err(ConfigError::ValidationError(
                "disk.system must be specified for installation".to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_immutable(&self, installed: &DiskConfig) -> Result<()> {
        if self.system != installed.system {
            return Err(ConfigError::ValidationError(format!(
                "disk.system cannot be changed after install (installed: '{}', requested: '{}')",
                installed.system, self.system
            )));
        }
        if self.data_disk() != installed.data_disk() {
            return Err(ConfigError::ValidationError(format!(
                "disk.data cannot be changed after install (installed: '{}', requested: '{}')",
                installed.data_disk(),
                self.data_disk()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_disk_fallback() {
        // ARRANGE
        let disk = crate::system::disk::DiskConfig {
            system: "/dev/sda".to_string(),
            ..Default::default()
        };

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sda");
        assert!(!disk.is_split());
    }

    #[test]
    fn data_disk_split() {
        // ARRANGE
        let disk = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some("/dev/sdb".to_string()),
        };

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sdb");
        assert!(disk.is_split());
    }

    #[test]
    fn empty_data_string_falls_back_to_system() {
        // ARRANGE
        let disk = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some(String::new()),
        };

        // ACT & ASSERT
        assert_eq!(disk.data_disk(), "/dev/sda");
        assert!(!disk.is_split());
    }

    #[test]
    fn validate_for_install_rejects_empty_system() {
        assert!(DiskConfig::default().validate_for_install().is_err());
    }

    #[test]
    fn validate_immutable_rejects_system_disk_change() {
        // ARRANGE
        let installed = DiskConfig {
            system: "/dev/sda".to_string(),
            data: None,
        };
        let requested = DiskConfig {
            system: "/dev/sdb".to_string(),
            data: None,
        };

        // ACT & ASSERT
        assert!(requested.validate_immutable(&installed).is_err());
    }

    #[test]
    fn validate_immutable_rejects_data_disk_change() {
        // ARRANGE
        let installed = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some("/dev/sdb".to_string()),
        };
        let requested = DiskConfig {
            system: "/dev/sda".to_string(),
            data: Some("/dev/sdc".to_string()),
        };

        // ACT & ASSERT
        assert!(requested.validate_immutable(&installed).is_err());
    }
}
