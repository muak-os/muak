//! LUKS key protection: TPM2 sealing, token management, and fallback to ESP file.

use std::path::Path;

use anyhow::{Context as _, Result};
use luks2::key::Passphrase;
use luks2::metadata::Tpm2Token;
use uki::measure::MeasuredSection;
use uki::section::canonical_rank;

use crate::efi;

/// Base directory for secrets.
pub(crate) const SECRETS_DIR: &str = "/run/state/secrets";

/// Result of sealing a LUKS key against a UKI.
pub(crate) enum SealResult {
    Sealed(Tpm2Token),
    EspKey,
}

/// Seals a LUKS key to TPM2 or signals writing it to the ESP as a fallback.
pub(crate) fn seal_luks_key(key: &[u8], sections: &[MeasuredSection]) -> Result<SealResult> {
    if tpm2::device::is_available(None) {
        let token = seal_to_token(key, sections).context("Failed to seal LUKS key to TPM2")?;
        return Ok(SealResult::Sealed(token));
    }

    Ok(SealResult::EspKey)
}

/// Unseals the LUKS key from a TPM2 token stored in the LUKS2 header.
///
/// # Errors
///
/// Returns an error when the STATE device is missing, the token or sealed blob
/// cannot be read, or TPM2 unsealing fails.
pub(crate) fn unseal_luks_key(state_device: Option<&str>) -> Result<Passphrase> {
    let state_device = state_device.context("STATE partition not found")?;
    let token = luks2::read_tpm2_token(state_device).context("Failed to read the TPM2 token")?;
    let blob_bytes = token
        .sealed_blob()
        .context("Failed to decode the TPM2 blob")?;
    let blob = tpm2::blob::Sealed::deserialize(&blob_bytes)
        .context("Failed to deserialize the TPM2 blob")?;
    let key = tpm2::operations::unseal(&blob).context("Failed to unseal the LUKS key from TPM2")?;

    println!("LUKS key unsealed from TPM2 for re-seal");

    Ok(key)
}

/// Resolves the LUKS key for an update by unsealing it from TPM2 when available,
/// and by reading the ESP key file otherwise.
///
/// # Errors
///
/// Returns an error when a TPM2 is available but the LUKS key cannot be unsealed.
/// A TPM2 unseal failure is fatal to the update.
pub(crate) fn resolve_luks_key(
    state_device: Option<&str>,
    efi_device: &str,
) -> Result<Option<Passphrase>> {
    if tpm2::device::is_available(None) {
        return unseal_luks_key(state_device).map(Some);
    }

    Ok(read_esp_key(efi_device))
}

/// Writes a LUKS2 TPM2 token to each of the given device paths.
pub(crate) fn write_token_to_devices(token: &Tpm2Token, devices: &[&str]) -> Result<()> {
    for dev in devices {
        luks2::write_tpm2_token(dev, token)
            .with_context(|| format!("Failed to write TPM2 token to {dev}"))?;
    }

    Ok(())
}

/// Seals a LUKS key to TPM2 PCR#11 predicted from the UKI and returns a LUKS2 token.
fn seal_to_token(luks_key: &[u8], sections: &[MeasuredSection]) -> Result<Tpm2Token> {
    let mut sections: Vec<(&str, &[u8; 32])> = sections
        .iter()
        .map(|section| (section.name, &section.hash))
        .collect();
    sections.sort_by_key(|&(name, _hash)| canonical_rank(name));
    let expected_pcr = tpm2::pcr::predict_pcr11(&sections);
    let sealed = tpm2::operations::seal(luks_key, &expected_pcr)
        .context("Failed to seal LUKS key to TPM2")?;

    Ok(Tpm2Token::new(
        &[11],
        &sealed.blob.serialize(),
        &sealed.policy_digest,
    ))
}

fn read_esp_key(efi_device: &str) -> Option<Passphrase> {
    if let Err(e) = efi::mount(efi_device) {
        eprintln!("Failed to mount EFI partition {efi_device} for the LUKS key: {e}");
        return None;
    }

    let path = Path::new(efi::MOUNT_POINT).join("luks");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Failed to read the LUKS key from {}: {e}", path.display());
            efi::unmount();
            return None;
        }
    };
    efi::unmount();

    match luks2::key::from_raw(bytes) {
        Ok(key) => Some(key),
        Err(error) => {
            eprintln!("Rejected LUKS key from the ESP: {error}");
            None
        }
    }
}
