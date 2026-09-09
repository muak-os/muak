//! Bridging of boot-image metadata into `/run/boot`.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};

/// Initramfs directory holding boot-image metadata.
pub(crate) const METADATA_DIR: &str = "/metadata";

/// Mount-independent home of booted-image metadata.
pub(crate) const BOOT_DIR: &str = "/run/boot";

/// Exposes the initramfs `metadata/` directory at `/run/boot`.
pub(crate) fn bridge() {
    if let Err(error) = bridge_from(Path::new(METADATA_DIR), Path::new(BOOT_DIR)) {
        kmsg::warn!("Failed to bridge boot metadata: {error:#}");
    }
}

fn bridge_from(metadata_dir: &Path, boot_dir: &Path) -> Result<()> {
    if !metadata_dir.exists() {
        kmsg::info!("Booted image carries no metadata directory");
        return Ok(());
    }

    fs::create_dir_all(boot_dir)
        .with_context(|| format!("failed to create {}", boot_dir.display()))?;

    let entries = fs::read_dir(metadata_dir)
        .with_context(|| format!("failed to read {}", metadata_dir.display()))?;

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to iterate {}", metadata_dir.display()))?;
        let source = entry.path();
        if !source.is_file() {
            continue;
        }

        let destination = boot_dir.join(entry.file_name());
        fs::copy(&source, &destination)
            .with_context(|| format!("failed to copy {}", source.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_copies_all_metadata_files() {
        // ARRANGE
        let initramfs = tempfile::tempdir().expect("tempdir");
        let metadata = initramfs.path().join("metadata");
        let boot_dir = initramfs.path().join("run/boot");
        std::fs::create_dir_all(&metadata).expect("mkdir");
        std::fs::write(metadata.join("profile.toml"), b"data").expect("write");

        // ACT
        bridge_from(&metadata, &boot_dir).expect("bridge");

        // ASSERT
        let copied = std::fs::read(boot_dir.join("profile.toml")).expect("read copy");
        assert_eq!(copied, b"data");
    }

    #[test]
    fn bridge_skips_missing_metadata_directory() {
        // ARRANGE
        let initramfs = tempfile::tempdir().expect("tempdir");
        let boot_dir = initramfs.path().join("run/boot");

        // ACT
        let result = bridge_from(&initramfs.path().join("metadata"), &boot_dir);

        // ASSERT
        assert!(result.is_ok(), "a missing metadata dir must not fail boot");
        assert!(!boot_dir.exists());
    }

    #[test]
    fn bridge_overwrites_stale_destinations() {
        // ARRANGE
        let initramfs = tempfile::tempdir().expect("tempdir");
        let metadata = initramfs.path().join("metadata");
        let boot_dir = initramfs.path().join("run/boot");
        std::fs::create_dir_all(&boot_dir).expect("mkdir");
        std::fs::write(boot_dir.join("profile.toml"), b"stale").expect("write stale");
        std::fs::create_dir_all(&metadata).expect("mkdir");
        std::fs::write(metadata.join("profile.toml"), b"fresh").expect("write fresh");

        // ACT
        bridge_from(&metadata, &boot_dir).expect("bridge");

        // ASSERT
        let copied = std::fs::read(boot_dir.join("profile.toml")).expect("read copy");
        assert_eq!(copied, b"fresh");
    }

    #[test]
    fn bridge_ignores_non_file_entries() {
        // ARRANGE
        let initramfs = tempfile::tempdir().expect("tempdir");
        let metadata = initramfs.path().join("metadata");
        let boot_dir = initramfs.path().join("run/boot");
        std::fs::create_dir_all(metadata.join("subdir")).expect("mkdir");

        // ACT
        bridge_from(&metadata, &boot_dir).expect("bridge");

        // ASSERT
        assert!(!boot_dir.join("subdir").exists());
    }
}
