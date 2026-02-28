//! Commits a validated update to the EFI partition.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::efi;
use crate::secrets;
use crate::uki::Uki;

/// Finds the EFI partition, builds the UKI, deploys it, then cleans up the staging directory.
pub async fn apply() -> Result<()> {
    println!("Validation succeeded, committing update.");

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;
    let data_device = disk::find_partition_by_partname("DATA").await;

    let staged = build_uki(state_device.as_deref(), data_device.as_deref())?;

    efi::deploy(&efi_device, &staged)?;

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}

/// Builds the UKI from the update staging directory and returns the path to the staged `.efi`.
fn build_uki(state_device: Option<&str>, data_device: Option<&str>) -> Result<PathBuf> {
    let mut uki = Uki::from_dir(Path::new(UPDATE_DIR));
    let staged = Path::new(UPDATE_DIR).join("staged.efi");

    let luks_key = secrets::resolve_luks_key(&mut uki, state_device, data_device)?;
    if let Some(key) = luks_key {
        uki = uki.with_luks_key(&key);
    }
    uki.build(&staged)?;

    if sysconfig::system().secureboot {
        let hierarchy = sbolt::keys::load_key_hierarchy(&Path::new(SECRETS_DIR).join("secureboot"))
            .context("Failed to load Secure Boot keys for UKI signing")?;
        Uki::sign(&staged, &hierarchy)?;
    }

    Ok(staged)
}
