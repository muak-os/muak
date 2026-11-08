//! Commits a validated update to the EFI partition.

use std::path::Path;

use ::disk::role::Role;
use anyhow::{Context as _, Result, bail};
use uki::measure::{self, MeasuredSection};

use super::{UPDATE_DIR, secureboot, snapshot};
use crate::disk::roles;
use crate::{cache, efi, medium, secrets};

/// Applies a staged update by deploying the UKI and resealing the LUKS key.
pub async fn apply() -> Result<()> {
    kmsg::info!("Validation succeeded, committing update");

    let (efi_device, state_device, ephemeral_device) = tokio::task::spawn_blocking(move || {
        (
            roles::device(Role::Esp),
            roles::device(Role::State),
            roles::device(Role::Ephemeral),
        )
    })
    .await
    .context("Failed to scan for partition devices")?;
    let efi_device = efi_device.ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let update_dir = Path::new(UPDATE_DIR);
    let assets_dir = update_dir.join("assets");
    let staged = update_dir.join("staged.efi");
    let signed_uki = assets_dir.join("uki.efi");

    std::fs::copy(&signed_uki, &staged)
        .with_context(|| format!("copy {} to {}", signed_uki.display(), staged.display()))?;

    let sections: Vec<MeasuredSection> =
        measure::from_file(&staged).context("Failed to read UKI sections from the staged image")?;

    let luks_key = match secrets::resolve_luks_key(state_device.as_deref()) {
        Some(key) => reseal_luks_key(
            &key,
            state_device.as_deref(),
            ephemeral_device.as_deref(),
            &sections,
        )?,
        None => None,
    };

    let secureboot_enabled = config::system::host()?.secureboot;

    if secureboot_enabled {
        secureboot::enroll_if_missing()?;
    } else {
        secureboot::unenroll_if_present()?;
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
        esp::arch::Arch::current().boot_path(),
        uki_len,
        &mut uki_file,
    )?;
    if let Some(ref key) = luks_key {
        efi::write_bytes(Path::new(efi::MOUNT_POINT), "luks", key)?;
    }
    efi::unmount();

    warn_on_version_mismatch();

    cache::enforce();

    if !secureboot_enabled {
        secureboot::remove_hierarchy();
    }

    remove_previous_config();

    if let Err(e) = std::fs::remove_dir_all(update_dir) {
        eprintln!("Failed to cleanup update work dir: {e}");
    }

    Ok(())
}

fn remove_previous_config() {
    if let Err(e) = std::fs::remove_file(snapshot::CONFIG_PREV_PATH)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        kmsg::error!("Failed to remove {}: {e}", snapshot::CONFIG_PREV_PATH);
    }
}

fn warn_on_version_mismatch() {
    let Ok(artifact) = medium::version() else {
        return;
    };
    let Ok(raw) = std::fs::read_to_string(config::system::CONFIG_PATH) else {
        return;
    };
    let Some(staged) = snapshot::host_version(&raw) else {
        return;
    };

    if artifact != staged {
        kmsg::warn!(
            "Booted artifact version {artifact} differs from staged config host.version {staged}"
        );
    }
}

fn reseal_luks_key(
    key: &[u8],
    state_device: Option<&str>,
    ephemeral_device: Option<&str>,
    sections: &[MeasuredSection],
) -> Result<Option<Vec<u8>>> {
    if !tpm2::device::is_available(None) {
        return Ok(Some(key.to_vec()));
    }

    let secrets::SealResult::Sealed(token) = secrets::seal_luks_key(key, sections)? else {
        bail!("TPM2 sealing unexpectedly returned an ESP key")
    };
    if ephemeral_device.is_none() {
        kmsg::warn!("EPHEMERAL partition not found; TPM2 token not re-sealed to EPHEMERAL");
    }
    let devices: Vec<&str> = [state_device, ephemeral_device]
        .into_iter()
        .flatten()
        .collect();
    secrets::write_token_to_devices(&token, &devices)?;
    kmsg::info!("LUKS key re-sealed to TPM2 with new PCR#11 values");

    Ok(None)
}
