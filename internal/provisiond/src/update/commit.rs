//! Commits a validated update to the EFI partition.

use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::sync;

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::uki::{self, Uki};

/// Mounts the EFI partition, deploys the new UKI, then unmounts.
pub async fn apply() -> Result<()> {
    println!("Validation succeeded, committing update.",);

    let efi_device = disk::find_partition_by_partname("EFI")
        .await
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let state_device = disk::find_partition_by_partname("STATE").await;
    let data_device = disk::find_partition_by_partname("DATA").await;

    let mount_point = "/run/mnt/efi";
    disk::mount_efi_partition(&efi_device, mount_point)?;

    let result = deploy_uki(mount_point, state_device.as_deref(), data_device.as_deref());

    disk::try_unmount(mount_point);

    result
}

/// Builds and signs the new UKI on the EFI partition and updates the system config.
fn deploy_uki(
    mount_point: &str,
    state_device: Option<&str>,
    data_device: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let mut components = Uki::from_dir(Path::new(UPDATE_DIR));
    let uki_path = uki::get_path(Path::new(mount_point))?;

    let luks_key = if tpm2::is_available() {
        match unseal_luks_key(state_device) {
            Some(key) => {
                let section_data = components
                    .read_section_data()
                    .context("Failed to read UKI sections for PCR prediction")?;

                let sections_ref: Vec<(&str, &[u8])> = section_data
                    .iter()
                    .map(|(name, data)| (name.as_str(), data.as_slice()))
                    .collect();

                let (sealed_blob, policy_hash) = tpm2::seal_to_pcr11(&key, &sections_ref)
                    .context("Failed to re-seal LUKS key to TPM2")?;

                let token = luks2::Tpm2Token {
                    r#type: "tpm2".to_string(),
                    keyslots: vec!["0".to_string()],
                    tpm2_pcrs: vec![11],
                    tpm2_hash_alg: "sha256".to_string(),
                    tpm2_blob: <base64ct::Base64 as base64ct::Encoding>::encode_string(
                        &sealed_blob,
                    ),
                    tpm2_policy_hash: <base64ct::Base64 as base64ct::Encoding>::encode_string(
                        &policy_hash,
                    ),
                };

                if let Some(dev) = state_device {
                    luks2::write_tpm2_token(dev, &token)
                        .context("Failed to write TPM2 token to STATE")?;
                }
                if let Some(dev) = data_device {
                    luks2::write_tpm2_token(dev, &token)
                        .context("Failed to write TPM2 token to DATA")?;
                }

                kmsg::info!("LUKS key re-sealed to TPM2 with new PCR#11 values");
                None
            }
            None => read_luks_key_from_cmdline(),
        }
    } else {
        read_luks_key_from_cmdline()
    };

    if let Some(key) = luks_key {
        components = components.with_luks_key(&key);
    }
    components.build_atomic(&uki_path)?;

    if sysconfig::system().secureboot {
        let hierarchy = sbolt::keys::load_key_hierarchy(&Path::new(SECRETS_DIR).join("secureboot"))
            .context("Failed to load Secure Boot keys for UKI signing")?;
        Uki::sign(&uki_path, &hierarchy)?;
    }

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    sync();

    Ok(())
}

/// Reads the LUKS key from the current `/proc/cmdline`.
fn read_luks_key_from_cmdline() -> Option<Vec<u8>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
    let encoded = cmdline
        .split_whitespace()
        .find(|t| t.starts_with("luks.key="))?
        .strip_prefix("luks.key=")?;
    <base64ct::Base64Unpadded as base64ct::Encoding>::decode_vec(encoded).ok()
}

/// Unseals the LUKS key from a TPM2 token in the LUKS2 header.
fn unseal_luks_key(state_device: Option<&str>) -> Option<Vec<u8>> {
    let token = luks2::read_tpm2_token(state_device?).ok()?;
    let blob = <base64ct::Base64 as base64ct::Encoding>::decode_vec(&token.tpm2_blob).ok()?;

    match tpm2::unseal_from_blob(&blob) {
        Ok(key) => {
            kmsg::info!("LUKS key unsealed from TPM2 for re-seal");
            Some(key)
        }
        Err(e) => {
            kmsg::warn!("TPM2 unseal failed during update: {}", e);
            None
        }
    }
}
