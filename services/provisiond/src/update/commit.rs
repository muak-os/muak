//! Commits a validated update to the EFI partition.

use std::path::Path;

use anyhow::{Context, Result};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::efi;
use crate::secrets;
use crate::uki::Uki;

/// Subdirectory for extracted board firmware files during update.
const FIRMWARE_DIR: &str = "/run/update/firmware";

/// Applies a staged update by building the UKI, enrolling Secure Boot keys if needed, and deploying.
pub async fn apply() -> Result<()> {
    kmsg::info!("Validation succeeded, committing update");

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;
    let data_device = disk::find_partition_by_partname("DATA").await;

    let mut uki = Uki::from_dir(Path::new(UPDATE_DIR));
    let staged = Path::new(UPDATE_DIR).join("staged.efi");

    secrets::resolve_luks_key(&mut uki, state_device.as_deref(), data_device.as_deref())?;

    let sb_hierarchy = if config::host().secureboot {
        Some(resolve_sb_hierarchy()?)
    } else {
        None
    };

    uki.build(&staged, sb_hierarchy.as_ref())?;

    if let Some(hier) = sb_hierarchy.as_ref() {
        let pk_missing = sbolt::efi::pk()
            .context("Failed to read PK from firmware")?
            .is_none();
        if pk_missing {
            sbolt::efi::enroll(hier).context("Failed to enroll Secure Boot keys into firmware")?;
        }
    }

    let firmware_dir = efi::resolve_firmware(config::host(), Path::new(FIRMWARE_DIR)).await?;

    efi::deploy(&efi_device, &staged, firmware_dir.as_deref())?;

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}

/// Returns the Secure Boot key hierarchy, generating and persisting it if not yet on disk.
fn resolve_sb_hierarchy() -> Result<sbolt::keys::hierarchy::Bundle> {
    let dir = Path::new(SECRETS_DIR).join("secureboot");
    if dir.exists() {
        sbolt::keys::storage::load_hierarchy(&dir).context("Failed to load Secure Boot keys")
    } else {
        let keys = sbolt::keys::hierarchy::Bundle::generate("Muak")
            .context("Failed to generate Secure Boot keys")?;
        sbolt::keys::storage::save_hierarchy(&keys, &dir)
            .context("Failed to save Secure Boot keys")?;
        Ok(keys)
    }
}
