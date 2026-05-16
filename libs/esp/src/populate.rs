//! Mounted-directory population helpers for ESP contents.

use std::path::Path;

use crate::{EspError, EspSpec, path};

/// Populates a mounted ESP directory from an `EspSpec`.
pub fn populate(spec: &EspSpec, esp_root: &Path) -> Result<(), EspError> {
    path::validate_spec(spec)?;

    for file in &spec.files {
        let rel_path = path::validate_relative_path(&file.path)?;
        let dest = esp_root.join(rel_path);
        let parent = dest
            .parent()
            .expect("validated ESP paths always have a parent");
        std::fs::create_dir_all(parent)?;
        std::fs::write(dest, &file.data)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::populate;
    use crate::{Arch, EspError, EspFile, EspSpec};

    #[test]
    fn populate_writes_all_files_to_directory() {
        // ARRANGE
        let spec = EspSpec::with_uki(
            Arch::X86_64,
            b"uki".to_vec(),
            vec![EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        populate(&spec, temp_dir.path()).expect("populate must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(temp_dir.path().join("EFI/BOOT/BOOTX64.EFI"))
                .expect("boot file must exist"),
            b"uki"
        );
        assert_eq!(
            std::fs::read(temp_dir.path().join("overlays/rpi/config.txt"))
                .expect("config file must exist"),
            b"arm_64bit=1"
        );
    }

    #[test]
    fn populate_rejects_invalid_paths() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "../escape".to_owned(),
                data: b"x".to_vec(),
            }],
        };
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let result = populate(&spec, temp_dir.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }
}
