//! Commits a validated update to the EFI partition.

use std::path::Path;

use anyhow::{Context, Result};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::efi;
use crate::secrets;
use crate::uki::Uki;

/// Applies a staged update by building the UKI, enrolling Secure Boot keys if needed, and deploying.
pub async fn apply() -> Result<()> {
    println!("Validation succeeded, committing update.");

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;
    let data_device = disk::find_partition_by_partname("DATA").await;

    let mut uki = Uki::from_dir(Path::new(UPDATE_DIR));
    let staged = Path::new(UPDATE_DIR).join("staged.efi");

    secrets::resolve_luks_key(&mut uki, state_device.as_deref(), data_device.as_deref())?;

    let first_enablement = is_first_secureboot_enablement();
    let sb_hierarchy = if sysconfig::system().secureboot {
        if first_enablement {
            Some(generate_sb_hierarchy()?)
        } else {
            Some(load_sb_hierarchy()?)
        }
    } else {
        None
    };

    uki.build(&staged, sb_hierarchy.as_ref())?;

    if let (true, Some(ref hierarchy)) = (first_enablement, sb_hierarchy.as_ref()) {
        sbolt::efi::enroll_keys(hierarchy)
            .context("Failed to enroll Secure Boot keys into firmware")?;
    }

    efi::deploy(&efi_device, &staged)?;

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}

/// Returns true if this is the first time Secure Boot is being enabled.
fn is_first_secureboot_enablement() -> bool {
    sysconfig::system().secureboot && !Path::new(SECRETS_DIR).join("secureboot").exists()
}

/// Generates a new Secure Boot key hierarchy and saves it to disk.
fn generate_sb_hierarchy() -> Result<sbolt::keys::KeyHierarchy> {
    let h = sbolt::keys::KeyHierarchy::generate("Muak")
        .context("Failed to generate Secure Boot keys")?;
    sbolt::keys::save_key_hierarchy(&h, &Path::new(SECRETS_DIR).join("secureboot"))
        .context("Failed to save Secure Boot keys")?;

    Ok(h)
}

/// Loads the Secure Boot key hierarchy from disk.
fn load_sb_hierarchy() -> Result<sbolt::keys::KeyHierarchy> {
    sbolt::keys::load_key_hierarchy(&Path::new(SECRETS_DIR).join("secureboot"))
        .context("Failed to load Secure Boot keys")
}
