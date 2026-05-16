//! EFI partition deployment of the Unified Kernel Image and board firmware.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use esp::{Arch, EspSpec};
use rustix::fs::sync;

use crate::constants::host_oci_arch;
use crate::disk;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

/// Pulls a board firmware OCI image and returns the variant subdirectory path.
pub async fn pull_firmware(firmware_ref: &str, variant: &str, dest: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("Failed to create firmware dir {}", dest.display()))?;

    koci::pull(firmware_ref, host_oci_arch(), dest, None)
        .await
        .with_context(|| format!("Failed to pull board firmware: {firmware_ref}"))?;

    let variant_dir = dest.join(variant);
    if !variant_dir.is_dir() {
        bail!(
            "Firmware variant '{}' not found in {}",
            variant,
            dest.display()
        );
    }

    Ok(variant_dir)
}

/// Resolves the firmware directory from host config, pulling the OCI image if configured.
pub async fn resolve_firmware(host: &config::HostConfig, dest: &Path) -> Result<Option<PathBuf>> {
    match (&host.firmware, &host.firmware_variant) {
        (Some(firmware_ref), Some(variant)) => {
            let dir = pull_firmware(firmware_ref, variant, dest).await?;
            Ok(Some(dir))
        }
        (None, None) => Ok(None),
        _ => bail!("host.firmware and host.firmware_variant must both be set or both be omitted"),
    }
}

/// Deploys the staged UKI and optional board firmware to the EFI partition.
pub fn deploy(efi_device: &str, staged_uki: &Path, firmware_dir: Option<&Path>) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let spec = build_esp_spec(staged_uki, firmware_dir)?;
    disk::mount_efi_partition(efi_device, MOUNT_POINT)?;
    esp::populate(&spec, Path::new(MOUNT_POINT)).context("Failed to populate EFI partition")?;

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}

/// Builds an ESP spec from the staged UKI and optional firmware files.
fn build_esp_spec(staged_uki: &Path, firmware_dir: Option<&Path>) -> Result<EspSpec> {
    let uki = std::fs::read(staged_uki)
        .with_context(|| format!("Failed to read staged UKI {}", staged_uki.display()))?;
    let extra_files = match firmware_dir {
        Some(dir) => esp::collect_tree(dir).context("Failed to collect firmware tree")?,
        None => Vec::new(),
    };
    Ok(EspSpec::with_uki(Arch::current(), uki, extra_files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_firmware_copies_recursive_tree_to_dest() {
        // ARRANGE
        let src = tempfile::tempdir().expect("create src dir");
        let dst = tempfile::tempdir().expect("create dst dir");
        std::fs::write(src.path().join("start4.elf"), b"gpu-fw").expect("write start4");
        std::fs::create_dir(src.path().join("subdir")).expect("create subdir");
        std::fs::write(src.path().join("subdir/config.txt"), b"kernel=u-boot.bin")
            .expect("write config");

        // ACT
        let files = esp::collect_tree(src.path()).expect("collect firmware");
        let spec = EspSpec::with_uki(Arch::current(), b"uki".to_vec(), files);
        esp::populate(&spec, dst.path()).expect("populate firmware");

        // ASSERT
        assert_eq!(
            std::fs::read(dst.path().join("start4.elf")).expect("read"),
            b"gpu-fw"
        );
        assert_eq!(
            std::fs::read(dst.path().join("subdir/config.txt")).expect("read"),
            b"kernel=u-boot.bin"
        );
        assert!(dst.path().join("subdir").exists());
    }

    #[test]
    fn copy_firmware_empty_dir_is_noop() {
        // ARRANGE
        let src = tempfile::tempdir().expect("create src dir");
        let dst = tempfile::tempdir().expect("create dst dir");

        // ACT
        let files = esp::collect_tree(src.path()).expect("collect firmware");
        let spec = EspSpec::with_uki(Arch::current(), b"uki".to_vec(), files);
        esp::populate(&spec, dst.path()).expect("populate firmware");

        // ASSERT
        let count = std::fs::read_dir(dst.path()).expect("read dst").count();
        assert_eq!(count, 1);
        assert!(dst.path().join("EFI").exists());
    }

    #[test]
    fn copy_firmware_fails_on_missing_src() {
        // ARRANGE / ACT
        let result = esp::collect_tree(Path::new("/nonexistent/firmware"));

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn build_spec_places_uki_at_fallback_path() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");

        // ACT
        let spec = build_esp_spec(&staged, None).expect("build spec");

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        assert_eq!(
            spec.files[0].path,
            format!("EFI/BOOT/{}", Arch::current().boot_filename())
        );
        assert_eq!(spec.files[0].data, b"uki-bytes");
    }
}
