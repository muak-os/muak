//! LUKS key protection: TPM2 sealing, token management, and the ESP key file.

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
    /// Plaintext key read from the ESP key file, only produced when no TPM2 is available.
    Esp(Passphrase),
}

/// Seals a LUKS key to TPM2 or signals writing it to the ESP when the TPM is unavailable.
pub(crate) fn seal_luks_key(key: &[u8], sections: &[MeasuredSection]) -> Result<SealResult> {
    if tpm2::device::is_available(None) {
        let token = seal_to_token(key, sections).context("Failed to seal LUKS key to TPM2")?;
        return Ok(SealResult::Sealed(token));
    }

    Ok(SealResult::EspKey)
}

/// Computes the TPM2 policy digest that an image measuring `sections` seals against.
pub(crate) fn expected_policy(sections: &[MeasuredSection]) -> [u8; 32] {
    tpm2::pcr::policy_digest(&sorted_sections(sections))
}

/// Resolves the LUKS key for an update: a TPM2 token when a TPM2 is available,
/// the ESP key file otherwise.
///
/// # Errors
///
/// Returns an error when the TPM2 is available but the STATE partition is
/// missing, or when the tokens cannot be read or unsealed.
pub(crate) fn resolve_luks_key(state_device: Option<&str>, efi_device: &str) -> Result<LuksKey> {
    if !tpm2::device::is_available(None) {
        return read_esp_key(efi_device)
            .map(LuksKey::Esp)
            .context("Failed to read the ESP LUKS key");
    }

    let state_device = state_device.context("STATE partition not found")?;

    Ok(LuksKey::Tpm(unseal_luks_key(state_device)?))
}

/// Adds a LUKS2 TPM2 token to each of the given device paths, keeping every
/// existing token in the header.
pub(crate) fn add_token_to_devices(token: &Tpm2Token, devices: &[&str]) -> Result<()> {
    for dev in devices {
        luks2::add_tpm2_token(dev, token)
            .with_context(|| format!("Failed to add TPM2 token to {dev}"))?;
    }

    Ok(())
}

fn unseal_luks_key(state_device: &str) -> Result<Passphrase> {
    let tokens = luks2::read_tpm2_tokens(state_device).context("Failed to read the TPM2 tokens")?;

    unseal_first(&tokens, &unseal_token)
}

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

fn unseal_token(token: &Tpm2Token) -> Result<Passphrase> {
    let blob_bytes = token
        .sealed_blob()
        .context("Failed to decode the TPM2 blob")?;
    let blob = tpm2::blob::Sealed::deserialize(&blob_bytes)
        .context("Failed to deserialize the TPM2 blob")?;

    tpm2::device::unseal(&blob).context("Failed to unseal the LUKS key from TPM2")
}

fn seal_to_token(luks_key: &[u8], sections: &[MeasuredSection]) -> Result<Tpm2Token> {
    let sections = sorted_sections(sections);
    let expected_pcr = tpm2::pcr::predict_pcr11(&sections);
    let sealed =
        tpm2::device::seal(luks_key, &expected_pcr).context("Failed to seal LUKS key to TPM2")?;

    Ok(Tpm2Token::new(
        &[11],
        &sealed.blob.serialize(),
        &sealed.policy_digest,
    ))
}

fn sorted_sections(sections: &[MeasuredSection]) -> Vec<(&str, &[u8; 32])> {
    let mut sorted: Vec<(&str, &[u8; 32])> = sections
        .iter()
        .map(|section| (section.name, &section.hash))
        .collect();
    sorted.sort_by_key(|&(name, _hash)| canonical_rank(name));

    sorted
}

fn read_esp_key(efi_device: &str) -> Result<Passphrase> {
    if let Err(error) = efi::mount(efi_device) {
        eprintln!("Failed to mount EFI partition {efi_device} for the LUKS key: {error:#}");
        return Err(error);
    }

    let path = Path::new(efi::MOUNT_POINT).join("luks");
    let result = std::fs::read(&path)
        .map_err(|error| {
            eprintln!(
                "Failed to read the LUKS key from {}: {error}",
                path.display()
            );
            error.into()
        })
        .and_then(|bytes| {
            luks2::key::from_raw(bytes).map_err(|error| {
                eprintln!("Rejected LUKS key from the ESP: {error:#}");
                error.into()
            })
        });
    efi::unmount();

    result
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
    fn add_token_to_devices_reports_the_failing_device() {
        // ARRANGE
        let devices = ["/nonexistent/muak-state", "/nonexistent/muak-ephemeral"];

        // ACT
        let message = format!(
            "{:#}",
            add_token_to_devices(&token(1), &devices)
                .expect_err("a missing device should not accept a token")
        );

        // ASSERT
        assert!(
            message.contains("Failed to add TPM2 token to /nonexistent/muak-state"),
            "message should name the failing device: {message}"
        );
    }

    #[test]
    fn add_token_to_devices_keeps_the_existing_token() {
        // ARRANGE
        let device = tempfile::NamedTempFile::new().expect("the temp device should be created");
        device
            .as_file()
            .set_len(16 * 1024 * 1024)
            .expect("the temp device should be sized for a LUKS2 header");
        let path = device
            .path()
            .to_str()
            .expect("the temp device path should be UTF-8");
        luks2::format(path, b"passphrase", "label").expect("the temp device should format");
        let first = token(1);
        let second = token(2);

        // ACT
        add_token_to_devices(&first, &[path]).expect("the first token should be added");
        add_token_to_devices(&second, &[path]).expect("the second token should be added");
        let tokens = luks2::read_tpm2_tokens(path).expect("the tokens should be readable");
        let markers: Vec<u8> = tokens.iter().map(marker).collect();

        // ASSERT
        assert_eq!(tokens.len(), 2, "both tokens should stay in the header");
        assert!(
            markers.contains(&1),
            "the existing token should survive: {markers:?}"
        );
        assert!(
            markers.contains(&2),
            "the new token should be added: {markers:?}"
        );
    }

    #[test]
    fn expected_policy_sorts_the_sections_canonically() {
        // ARRANGE
        let cmdline_hash = [0x11_u8; 32];
        let kernel_hash = [0x22_u8; 32];
        let out_of_order = [
            MeasuredSection {
                name: uki::section::KERNEL,
                hash: kernel_hash,
            },
            MeasuredSection {
                name: uki::section::CMDLINE,
                hash: cmdline_hash,
            },
        ];
        let canonical: [(&str, &[u8; 32]); 2] = [
            (uki::section::CMDLINE, &cmdline_hash),
            (uki::section::KERNEL, &kernel_hash),
        ];

        // ACT
        let policy = expected_policy(&out_of_order);

        // ASSERT
        assert_eq!(policy, tpm2::pcr::policy_digest(&canonical));
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
