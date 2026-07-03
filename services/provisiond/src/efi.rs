//! EFI partition deployment of the Unified Kernel Image and overlay assets.

use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use esp::model::{Arch, EspFile, EspSpec, EspSpecBuilder};
use rustix::fs::sync;
use wizard::build::source::OverlayEntry;

use crate::disk;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

/// Whether the LUKS key was sealed to TPM2 or must be written to the ESP.
#[must_use]
pub enum LuksKey {
    /// Key was sealed to TPM
    TpmSealed,
    /// Key must be placed on the ESP as a file.
    EspKey(Vec<u8>),
}

pub fn deploy(
    efi_device: &str,
    uki_file: &mut File,
    uki_len: u64,
    overlay_entries: Vec<OverlayEntry>,
    luks: LuksKey,
) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    with_esp(uki_file, uki_len, overlay_entries, luks, |spec| {
        disk::mount_efi_partition(efi_device, MOUNT_POINT)?;
        esp::populate::write(spec.files_mut(), Path::new(MOUNT_POINT))
            .context("Failed to populate EFI partition")?;

        sync();
        disk::try_unmount(MOUNT_POINT);

        Ok(())
    })
}

fn with_esp(
    uki_file: &mut File,
    uki_len: u64,
    overlay_entries: Vec<OverlayEntry>,
    luks: LuksKey,
    write: impl FnOnce(&mut EspSpec<'_>) -> Result<()>,
) -> Result<()> {
    let mut overlay_cursors: Vec<Cursor<Arc<[u8]>>> = Vec::with_capacity(overlay_entries.len());
    for entry in &overlay_entries {
        overlay_cursors.push(Cursor::new(Arc::clone(&entry.data)));
    }

    let mut luks_cursor: Option<Cursor<Arc<[u8]>>> = None;
    if let LuksKey::EspKey(ref key) = luks {
        luks_cursor = Some(Cursor::new(Arc::from(key.as_slice())));
    }

    let mut builder = EspSpecBuilder::default();
    let boot = EspFile::boot(Arch::current(), uki_file, uki_len);
    builder = builder
        .add_file(boot)
        .context("Failed to add staged UKI to ESP spec")?;

    for (cursor, entry) in overlay_cursors.iter_mut().zip(&overlay_entries) {
        builder = builder
            .add_file(EspFile {
                path: entry.path.clone(),
                reader: cursor,
                size: entry.size,
            })
            .context("Failed to add overlay file")?;
    }

    if let Some(ref mut cursor) = luks_cursor {
        let size = u64::try_from(cursor.get_ref().len()).unwrap_or(u64::MAX);
        builder = builder
            .add_file(EspFile {
                path: "luks".to_owned(),
                reader: cursor,
                size,
            })
            .context("Failed to add LUKS key")?;
    }

    let mut spec = builder.build().context("Failed to build ESP spec")?;

    write(&mut spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_rejects_nonexistent_device() {
        // ARRANGE
        let mut uki_file = tempfile::tempfile().expect("create temp file");
        let uki_len = uki_file.metadata().expect("metadata").len();

        // ACT
        let result = deploy(
            "/nonexistent/efi",
            &mut uki_file,
            uki_len,
            vec![],
            LuksKey::TpmSealed,
        );

        // ASSERT
        assert!(result.is_err());
    }
}
