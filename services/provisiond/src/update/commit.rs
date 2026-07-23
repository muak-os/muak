//! Commits a validated update to the EFI partition.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use esp::arch::Arch;
use wizard::build::SectionInfo;

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::efi;
use crate::secrets;

/// Applies a staged update by deploying the UKI and resealing the LUKS key.
pub async fn apply() -> Result<()> {
    kmsg::info!("Validation succeeded, committing update");

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;

    let update_dir = Path::new(UPDATE_DIR);
    let assets_dir = update_dir.join("assets");
    let staged = update_dir.join("staged.efi");
    let signed_uki = assets_dir.join("uki.efi");

    std::fs::copy(&signed_uki, &staged)
        .with_context(|| format!("copy {} to {}", signed_uki.display(), staged.display()))?;

    let sections: Vec<SectionInfo> = {
        let path = assets_dir.join("sections.json");
        let data = fs::read_to_string(&path)
            .with_context(|| format!("read sections from {}", path.display()))?;

        serde_json::from_str(&data).context("Failed to deserialize UKI sections")?
    };

    let luks_key = if let Some(key) = secrets::resolve_luks_key(state_device.as_deref()) {
        if tpm2::device::is_available(None) {
            let token = match secrets::seal_luks_key(&key, &sections)? {
                secrets::SealResult::Sealed(token) => token,
                _ => unreachable!(),
            };
            let devices: Vec<&str> = [state_device.as_deref()].into_iter().flatten().collect();
            secrets::write_token_to_devices(&token, &devices)?;
            kmsg::info!("LUKS key re-sealed to TPM2 with new PCR#11 values");
            None
        } else {
            Some(key.to_vec())
        }
    } else {
        None
    };

    // Enroll PK if missing (signing keys are on STATE from prepare phase)
    if config::host().secureboot {
        let pk_missing = sbolt::efi::pk()
            .context("Failed to read PK from firmware")?
            .is_none();
        if pk_missing {
            let dir = Path::new(SECRETS_DIR).join("secureboot");
            let hierarchy = sbolt::keys::storage::load_hierarchy(&dir)
                .context("Failed to load Secure Boot keys for enrollment")?;
            sbolt::efi::enroll(&hierarchy)
                .context("Failed to enroll Secure Boot keys into firmware")?;
        }
    }

    let mut uki_file = std::fs::File::open(&staged)
        .with_context(|| format!("Failed to open staged UKI {}", staged.display()))?;
    let uki_len = uki_file
        .metadata()
        .with_context(|| format!("Failed to get metadata for {}", staged.display()))?
        .len();

    efi::mount(&efi_device)?;
    efi::write_file(
        Path::new(efi::MOUNT_POINT),
        Arch::current().boot_path(),
        uki_len,
        &mut uki_file,
    )?;
    if let Some(ref key) = luks_key {
        efi::write_bytes(Path::new(efi::MOUNT_POINT), "luks", key)?;
    }
    efi::unmount();

    if let Err(e) = std::fs::remove_dir_all(update_dir) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}
