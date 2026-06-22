//! Commits a validated update to the EFI partition.

use std::path::Path;

use anyhow::{Context, Result};

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

    let mut esp_files: Vec<esp::EspFile> = vec![];
    if let Some(key) = secrets::resolve_luks_key(state_device.as_deref()) {
        if tpm2::is_available() {
            let token = match secrets::seal_luks_key(&key, &[])? {
                secrets::SealResult::Sealed(token) => token,
                _ => unreachable!(),
            };
            let devices: Vec<&str> = [state_device.as_deref()].into_iter().flatten().collect();
            secrets::write_token_to_devices(&token, &devices)?;
            kmsg::info!("LUKS key re-sealed to TPM2 with new PCR#11 values");
        } else {
            esp_files.push(esp::EspFile {
                path: "luks".into(),
                data: key.to_vec(),
            });
        }
    }

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

    efi::deploy(&efi_device, &staged, &esp_files)?;

    if let Err(e) = std::fs::remove_dir_all(update_dir) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}
