//! Commits a validated update to the EFI partition.

use std::path::Path;

use anyhow::{Context, Result};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::efi;
use crate::secrets;

/// Applies a staged update by signing and deploying the UKI, and writing the LUKS key.
pub async fn apply() -> Result<()> {
    kmsg::info!("Validation succeeded, committing update");

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;

    let update_dir = Path::new(UPDATE_DIR);
    let assets_dir = update_dir.join("assets");
    let uki_path = assets_dir.join("uki.efi");
    let staged = update_dir.join("staged.efi");

    let sb_hierarchy = if config::host().secureboot {
        Some(resolve_sb_hierarchy()?)
    } else {
        None
    };

    let uki_bytes =
        std::fs::read(&uki_path).with_context(|| format!("read UKI {}", uki_path.display()))?;

    sign_uki(&uki_bytes, &staged, sb_hierarchy.as_ref())?;

    let mut esp_files: Vec<esp::EspFile> = vec![];
    if let Some(key) = secrets::resolve_luks_key(state_device.as_deref()) {
        if tpm2::is_available() {
            let token = match secrets::seal_luks_key(&key, &uki_bytes, &[])? {
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

    efi::deploy(&efi_device, &staged, &esp_files)?;

    if let Err(e) = std::fs::remove_dir_all(update_dir) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    Ok(())
}

fn sign_uki(
    uki_bytes: &[u8],
    output: &Path,
    sb_hierarchy: Option<&sbolt::keys::hierarchy::Bundle>,
) -> Result<()> {
    {
        let mut file = std::fs::File::create(output)
            .with_context(|| format!("create UKI {}", output.display()))?;

        if let Some(hierarchy) = sb_hierarchy {
            sbolt::pe::signature::sign(
                uki_bytes,
                &hierarchy.db.signer,
                &hierarchy.db.certificate,
                &mut file,
            )
            .context("Failed to sign UKI")?;
        } else {
            use std::io::Write;
            file.write_all(uki_bytes)
                .with_context(|| format!("write UKI {}", output.display()))?;
        }
    }

    if let Some(hierarchy) = sb_hierarchy {
        let pk_missing = sbolt::efi::pk()
            .context("Failed to read PK from firmware")?
            .is_none();
        if pk_missing {
            sbolt::efi::enroll(hierarchy)
                .context("Failed to enroll Secure Boot keys into firmware")?;
        }
    }

    Ok(())
}

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
