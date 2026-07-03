//! EFI partition deployment of the Unified Kernel Image and overlay assets.

use std::path::Path;

use anyhow::{Context, Result, bail};
use esp::model::{Arch, EspFile, EspSpec, EspSpecBuilder};
use rustix::fs::sync;

use crate::disk;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

pub fn deploy(efi_device: &str, staged_uki: &Path, esp_files: Vec<EspFile>) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let mut spec = build_esp_spec(staged_uki, esp_files)?;
    disk::mount_efi_partition(efi_device, MOUNT_POINT)?;
    esp::populate::write(spec.files_mut(), Path::new(MOUNT_POINT))
        .context("Failed to populate EFI partition")?;

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}

fn build_esp_spec(staged_uki: &Path, esp_files: Vec<EspFile>) -> Result<EspSpec> {
    let uki_file = std::fs::File::open(staged_uki)
        .with_context(|| format!("Failed to open staged UKI {}", staged_uki.display()))?;
    let uki_len = uki_file
        .metadata()
        .with_context(|| format!("Failed to get metadata for {}", staged_uki.display()))?
        .len();
    let boot = EspFile::boot(Arch::current(), uki_file, uki_len);

    EspSpecBuilder::default()
        .add_file(boot)
        .context("Failed to add staged UKI to ESP spec")?
        .add_files(esp_files)
        .context("Failed to add overlay files to ESP spec")?
        .build()
        .context("Failed to build ESP spec")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn build_spec_includes_staged_uki() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");

        // ACT
        let spec = build_esp_spec(&staged, vec![]).expect("build spec");

        // ASSERT
        assert_eq!(spec.files().len(), 1);
    }

    #[test]
    fn build_spec_includes_overlay_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("create temp dir");
        let staged = dir.path().join("staged.efi");
        std::fs::write(&staged, b"uki-bytes").expect("write staged UKI");
        let overlay = vec![EspFile {
            path: "dtb/rpi.dtb".to_owned(),
            reader: Box::new(Cursor::new(b"dtb".to_vec())),
            size: 3,
        }];

        // ACT
        let spec = build_esp_spec(&staged, overlay).expect("build spec");

        // ASSERT
        assert_eq!(spec.files().len(), 2);
    }
}
