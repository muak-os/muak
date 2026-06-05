//! EFI partition deployment of the Unified Kernel Image.

use std::path::Path;

use anyhow::{Context, Result, bail};
use esp::{Arch, EspSpec, EspSpecBuilder};
use rustix::fs::sync;

use crate::disk;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

/// Deploys the staged UKI to the EFI partition.
pub fn deploy(efi_device: &str, staged_uki: &Path) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let spec = build_esp_spec(staged_uki)?;
    disk::mount_efi_partition(efi_device, MOUNT_POINT)?;
    esp::populate(&spec, Path::new(MOUNT_POINT)).context("Failed to populate EFI partition")?;

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}

/// Builds an ESP spec from the staged UKI.
fn build_esp_spec(staged_uki: &Path) -> Result<EspSpec> {
    let uki = std::fs::read(staged_uki)
        .with_context(|| format!("Failed to read staged UKI {}", staged_uki.display()))?;
    EspSpecBuilder::default()
        .with_uki(Arch::current(), uki)
        .context("Failed to add staged UKI to ESP spec")?
        .build()
        .context("Failed to build ESP spec")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spec_includes_staged_uki() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");

        // ACT
        let spec = build_esp_spec(&staged).expect("build spec");

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].data, b"uki-bytes");
    }
}
