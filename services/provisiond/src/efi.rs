//! EFI partition deployment of the Unified Kernel Image and board firmware.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use esp::{Arch, EspSpec, EspSpecBuilder};
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
    let builder = EspSpecBuilder::default()
        .with_uki(Arch::current(), uki)
        .context("Failed to add staged UKI to ESP spec")?;

    let builder = match firmware_dir {
        Some(dir) => builder
            .add_files(esp::collect_tree(dir).context("Failed to collect firmware tree")?)
            .context("Failed to add firmware files to ESP spec")?,
        None => builder,
    };

    builder.build().context("Failed to build ESP spec")
}

#[cfg(test)]
mod tests {
    use config::HostConfig;

    use super::*;

    #[test]
    fn build_spec_includes_staged_uki_and_firmware_payloads() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        let firmware_dir = dir.path().join("firmware");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");
        std::fs::create_dir(&firmware_dir).expect("create firmware dir");
        std::fs::write(firmware_dir.join("start4.elf"), b"gpu-fw").expect("write firmware");

        // ACT
        let spec = build_esp_spec(&staged, Some(&firmware_dir)).expect("build spec");

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        assert!(spec.files.iter().any(|file| file.data == b"uki-bytes"));
        assert!(spec.files.iter().any(|file| file.data == b"gpu-fw"));
    }

    #[test]
    fn build_spec_without_firmware_only_includes_staged_uki() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");

        // ACT
        let spec = build_esp_spec(&staged, None).expect("build spec");

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].data, b"uki-bytes");
    }

    #[test]
    fn build_spec_wraps_firmware_collection_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");

        // ACT
        let result = build_esp_spec(&staged, Some(&dir.path().join("missing-firmware")));

        // ASSERT
        let error = result.expect_err("missing firmware dir must fail");
        assert!(
            error
                .to_string()
                .contains("Failed to collect firmware tree")
        );
    }

    #[tokio::test]
    async fn resolve_firmware_returns_none_when_host_has_no_firmware() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let host = HostConfig::default();

        // ACT
        let firmware = resolve_firmware(&host, dir.path())
            .await
            .expect("missing firmware should be allowed");

        // ASSERT
        assert_eq!(firmware, None);
    }

    #[tokio::test]
    async fn resolve_firmware_requires_reference_and_variant_together() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");

        let host_with_ref_only = HostConfig {
            firmware: Some("oci://firmware".to_owned()),
            ..HostConfig::default()
        };
        let host_with_variant_only = HostConfig {
            firmware_variant: Some("rpi".to_owned()),
            ..HostConfig::default()
        };

        // ACT
        let ref_only_result = resolve_firmware(&host_with_ref_only, dir.path()).await;
        let variant_only_result = resolve_firmware(&host_with_variant_only, dir.path()).await;

        // ASSERT
        assert!(ref_only_result.is_err());
        assert!(variant_only_result.is_err());
    }
}
