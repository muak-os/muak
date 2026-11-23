//! LUKS key protection: TPM2 sealing, token management, and fallback to ESP file.

use std::path::Path;

use anyhow::{Context as _, Result};
use luks2::key::Passphrase;
use luks2::metadata::tokens::Tpm2Token;
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

/// Origin of the LUKS key resolved for an update.
pub(crate) enum LuksKey {
    /// Key unsealed from a LUKS2 TPM2 token.
    Tpm(Passphrase),
    /// Plaintext key read from the ESP key file.
    Esp(Passphrase),
}

/// Seals a LUKS key to TPM2 or signals writing it to the ESP as a fallback.
pub(crate) fn seal_luks_key(key: &[u8], sections: &[MeasuredSection]) -> Result<SealResult> {
    if tpm2::device::is_available(None) {
        let token = seal_to_token(key, sections).context("Failed to seal LUKS key to TPM2")?;
        return Ok(SealResult::Sealed(token));
    }

    Ok(SealResult::EspKey)
}

/// Resolves the LUKS key for an update, preferring a TPM2 token and falling back
/// to the ESP key file.
///
/// # Errors
///
/// Returns an error when the TPM2 is available but the STATE partition is
/// missing, or when the tokens cannot be read. A TPM2 unseal failure falls back
/// to the ESP key, and it is an error when neither the tokens nor the ESP key
/// file yield a key while the TPM2 is available.
pub(crate) fn resolve_luks_key(
    state_device: Option<&str>,
    efi_device: &str,
) -> Result<Option<LuksKey>> {
    if !tpm2::device::is_available(None) {
        return Ok(read_esp_key(efi_device).map(LuksKey::Esp));
    }

    let state_device = state_device.context("STATE partition not found")?;

    match unseal_luks_key(state_device) {
        Ok(key) => Ok(Some(LuksKey::Tpm(key))),
        Err(error) => match read_esp_key(efi_device) {
            Some(key) => {
                kmsg::warn!("LUKS key read from the ESP, TPM2 unseal abandoned: {error:#}");
                Ok(Some(LuksKey::Esp(key)))
            }
            None => Err(error),
        },
    }
}

/// Writes a LUKS2 TPM2 token to each of the given device paths.
pub(crate) fn write_token_to_devices(token: &Tpm2Token, devices: &[&str]) -> Result<()> {
    for dev in devices {
        luks2::set_tpm2_tokens(dev, std::slice::from_ref(token))
            .with_context(|| format!("Failed to write TPM2 token to {dev}"))?;
    }

    Ok(())
}

// Reads the LUKS2 header of the STATE device and unseals the first token that succeeds.
fn unseal_luks_key(state_device: &str) -> Result<Passphrase> {
    let tokens = luks2::read_tpm2_tokens(state_device).context("Failed to read the TPM2 tokens")?;

    unseal_first(&tokens, &unseal_token)
}

// Tries every token in order and returns the key of the first token that unseals.
fn unseal_first(
    tokens: &[Tpm2Token],
    unseal: &dyn Fn(&Tpm2Token) -> Result<Passphrase>,
) -> Result<Passphrase> {
    let mut last_error = None;

    for (index, token) in tokens.iter().enumerate() {
        match unseal(token) {
            Ok(key) => {
                kmsg::info!("LUKS key unsealed from TPM2 for re-seal");
                return Ok(key);
            }
            Err(error) => {
                kmsg::info!(
                    "TPM2 token {} failed to unseal: {error:#}",
                    index.saturating_add(1)
                );
                last_error = Some(error);
            }
        }
    }

    let attempted = tokens.len();
    match last_error {
        Some(error) => Err(error.context(format!("TPM2 unseal failed ({attempted} tokens tried)"))),
        None => Err(anyhow::anyhow!(
            "TPM2 unseal failed: no TPM2 token in the LUKS2 header"
        )),
    }
}

// Decodes and unseals a single LUKS2 TPM2 token.
fn unseal_token(token: &Tpm2Token) -> Result<Passphrase> {
    let blob_bytes = token
        .sealed_blob()
        .context("Failed to decode the TPM2 blob")?;
    let blob = tpm2::blob::Sealed::deserialize(&blob_bytes)
        .context("Failed to deserialize the TPM2 blob")?;

    tpm2::operations::unseal(&blob).context("Failed to unseal the LUKS key from TPM2")
}

// Seals a LUKS key to TPM2 PCR#11 predicted from the UKI and returns a LUKS2 token.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn token(marker: u8) -> Tpm2Token {
        Tpm2Token::new(&[11], &[marker], &[0_u8; 32])
    }

    fn key(marker: u8) -> Passphrase {
        luks2::key::from_raw(vec![marker; luks2::key::VOLUME_KEY_SIZE])
            .expect("test key should have the volume key length")
    }

    fn marker(token: &Tpm2Token) -> u8 {
        token
            .sealed_blob()
            .expect("the test token should seal a blob")[0]
    }

    // Fails on the first marker and unseals every other marker into its own key.
    fn stale_first(token: &Tpm2Token) -> Result<Passphrase> {
        let marker = marker(token);

        if marker == 1 {
            return Err(anyhow::anyhow!("stale PCR"));
        }

        Ok(key(marker))
    }

    #[test]
    fn unseal_first_returns_the_first_token_key() {
        // ARRANGE
        let tokens = [token(1), token(2)];

        // ACT
        let result = unseal_first(&tokens, &|token: &Tpm2Token| -> Result<Passphrase> {
            Ok(key(marker(token)))
        });

        // ASSERT
        assert_eq!(
            result.expect("the first token should unseal").as_slice(),
            key(1).as_slice()
        );
    }

    #[test]
    fn unseal_first_continues_after_a_failing_token() {
        // ARRANGE
        let tokens = [token(1), token(2)];

        // ACT
        let result = unseal_first(&tokens, &stale_first);

        // ASSERT
        assert_eq!(
            result.expect("the second token should unseal").as_slice(),
            key(2).as_slice()
        );
    }

    #[test]
    fn unseal_first_fails_when_every_token_fails() {
        // ARRANGE
        let tokens = [token(1), token(2)];
        let unseal =
            |_token: &Tpm2Token| -> Result<Passphrase> { Err(anyhow::anyhow!("stale PCR")) };

        // ACT
        let message = format!(
            "{:#}",
            unseal_first(&tokens, &unseal).expect_err("no token should unseal")
        );

        // ASSERT
        assert!(
            message.contains("TPM2 unseal failed"),
            "message should stay greppable: {message}"
        );
        assert!(
            message.contains("2 tokens tried"),
            "message should name the token count: {message}"
        );
    }
}
