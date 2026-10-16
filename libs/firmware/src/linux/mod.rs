//! Linux firmware implementation over sysfs and efivarfs.

mod nvram;
mod platform;

use std::path::{Path, PathBuf};

/// Linux firmware implementation reading sysfs and efivarfs.
#[derive(Debug, Clone)]
pub struct Linux {
    sysroot: PathBuf,
    efi_firmware_path: PathBuf,
    efivarfs_path: PathBuf,
}

impl Linux {
    /// Creates an implementation rooted at the system `/sys`.
    #[must_use]
    pub fn new() -> Self {
        Self::rooted(Path::new("/sys"))
    }

    /// Creates an implementation rooted at `sysroot`, for tests and alternate mounts.
    #[must_use]
    pub fn rooted(sysroot: &Path) -> Self {
        let firmware = sysroot.join("firmware").join("efi");

        Self {
            sysroot: sysroot.to_path_buf(),
            efivarfs_path: firmware.join("efivars"),
            efi_firmware_path: firmware,
        }
    }
}

impl Default for Linux {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_system_efi_paths() {
        // ARRANGE
        let system = Linux::new();
        let default = Linux::default();

        // ACT & ASSERT
        assert_eq!(system.efi_firmware_path, PathBuf::from("/sys/firmware/efi"));
        assert_eq!(
            system.efivarfs_path,
            PathBuf::from("/sys/firmware/efi/efivars")
        );
        assert_eq!(default.efi_firmware_path, system.efi_firmware_path);
        assert_eq!(default.efivarfs_path, system.efivarfs_path);
    }

    #[test]
    fn rooted_builds_nested_paths() {
        // ARRANGE
        let system = Linux::rooted(Path::new("/tmp/sysroot"));

        // ACT & ASSERT
        assert_eq!(
            system.efi_firmware_path,
            PathBuf::from("/tmp/sysroot/firmware/efi")
        );
        assert_eq!(
            system.efivarfs_path,
            PathBuf::from("/tmp/sysroot/firmware/efi/efivars")
        );
    }
}
