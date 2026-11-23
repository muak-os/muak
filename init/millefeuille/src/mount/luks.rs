//! LUKS key resolution.

use anyhow::{Context as _, Result};
use blockdev::discover;
use disk::role::Role;
use luks2::key::Passphrase;
use luks2::metadata::tokens::Tpm2Token;
use rustix::fs::{AtFlags, CWD, Mode, mkdirat, unlinkat};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use tpm2::{blob, device, operations};

const ESP_MOUNT: &str = "/run/efi";
const ESP_KEY_NAME: &str = "luks";
const ESP_FSTYPE: &str = "vfat";

// Resolves the LUKS key for the given device: every TPM2 token when the TPM is available, the ESP key file otherwise.
pub(super) fn resolve_key(device: &str) -> Result<Option<Passphrase>> {
    if !device::is_available(None) {
        return Ok(try_esp_key());
    }

    let tokens = luks2::read_tpm2_tokens(device).context("Failed to read the LUKS2 tokens")?;

    unseal_tokens(&tokens, &unseal_tpm2).map(Some)
}

fn unseal_tokens(
    tokens: &[Tpm2Token],
    unseal: &dyn Fn(&[u8]) -> Result<Passphrase>,
) -> Result<Passphrase> {
    let Some((first, rest)) = tokens.split_first() else {
        return Err(anyhow::anyhow!(
            "TPM2 unseal failed: no TPM2 token in the LUKS2 header"
        ));
    };

    let mut last_error = match try_token(first, 1, unseal) {
        Ok(key) => return Ok(key),
        Err(error) => error,
    };

    for (index, token) in rest.iter().enumerate() {
        match try_token(token, index.saturating_add(2), unseal) {
            Ok(key) => return Ok(key),
            Err(error) => last_error = error,
        }
    }

    let attempted = tokens.len();

    Err(last_error.context(format!("TPM2 unseal failed ({attempted} tokens tried)")))
}

fn try_token(
    token: &Tpm2Token,
    number: usize,
    unseal: &dyn Fn(&[u8]) -> Result<Passphrase>,
) -> Result<Passphrase> {
    let result = token
        .sealed_blob()
        .context("Failed to decode TPM2 blob from LUKS token")
        .and_then(|blob| unseal(&blob));

    match result {
        Ok(key) => {
            kmsg::info!("LUKS key unsealed from TPM2");
            Ok(key)
        }
        Err(error) => {
            kmsg::info!("TPM2 token {number} failed to unseal: {:#}", error);
            Err(error)
        }
    }
}

fn unseal_tpm2(bytes: &[u8]) -> Result<Passphrase> {
    let sealed = blob::Sealed::deserialize(bytes).context("Failed to deserialize TPM2 blob")?;

    Ok(operations::unseal(&sealed)?)
}

fn try_esp_key() -> Option<Passphrase> {
    let candidates = discover::find_by_partname(Role::Esp.gpt_name());
    if candidates.is_empty() {
        kmsg::info!("LUKS key file skipped: ESP partition not found");
        return None;
    }

    let mut last_error = None;
    for esp in candidates {
        match read_esp_key(&esp) {
            Ok(key) => {
                kmsg::info!("LUKS key read from ESP {}", esp);
                return Some(key);
            }
            Err(error) => last_error = Some((esp, error)),
        }
    }

    if let Some((esp, error)) = last_error {
        kmsg::info!("LUKS key file skipped on ESP {}: {:#}", esp, error);
    }

    None
}

fn read_esp_key(device: &str) -> Result<Passphrase> {
    if let Err(error) = mkdirat(CWD, ESP_MOUNT, Mode::from_bits_truncate(0o755))
        && error != Errno::EXIST
    {
        return Err(error).with_context(|| format!("Failed to create {ESP_MOUNT}"));
    }

    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV;
    if let Err(error) = mount(device, ESP_MOUNT, ESP_FSTYPE, flags, None) {
        remove_mountpoint();
        return Err(error).with_context(|| format!("Failed to mount ESP {device} read-only"));
    }

    let result = read_key_file();
    if let Err(error) = unmount(ESP_MOUNT, UnmountFlags::empty()) {
        kmsg::warn!("Failed to unmount {}: {}", ESP_MOUNT, error);
    }
    remove_mountpoint();

    result
}

fn read_key_file() -> Result<Passphrase> {
    let path = format!("{ESP_MOUNT}/{ESP_KEY_NAME}");
    let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {path}"))?;

    Ok(luks2::key::from_raw(bytes)?)
}

fn remove_mountpoint() {
    if let Err(error) = unlinkat(CWD, ESP_MOUNT, AtFlags::REMOVEDIR)
        && error != Errno::NOENT
    {
        kmsg::warn!("Failed to remove {}: {}", ESP_MOUNT, error);
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

    // Fails on the first marker and unseals every other marker into its own key.
    fn stale_first(bytes: &[u8]) -> Result<Passphrase> {
        let marker = *bytes.first().expect("test blob should carry a marker");

        if marker == 1 {
            return Err(anyhow::anyhow!("stale PCR"));
        }

        Ok(key(marker))
    }

    #[test]
    fn unseal_tokens_returns_the_first_token_key() {
        // ARRANGE
        let tokens = [token(1), token(2)];

        // ACT
        let result = unseal_tokens(&tokens, &|bytes: &[u8]| -> Result<Passphrase> {
            Ok(key(bytes[0]))
        });

        // ASSERT
        assert_eq!(
            result.expect("the first token should unseal").as_slice(),
            key(1).as_slice()
        );
    }

    #[test]
    fn unseal_tokens_continues_after_a_failing_token() {
        // ARRANGE
        let tokens = [token(1), token(2)];

        // ACT
        let result = unseal_tokens(&tokens, &stale_first);

        // ASSERT
        assert_eq!(
            result.expect("the second token should unseal").as_slice(),
            key(2).as_slice()
        );
    }

    #[test]
    fn unseal_tokens_fails_when_every_token_fails() {
        // ARRANGE
        let tokens = [token(1), token(2)];
        let unseal = |_bytes: &[u8]| -> Result<Passphrase> { Err(anyhow::anyhow!("stale PCR")) };

        // ACT
        let result = unseal_tokens(&tokens, &unseal);

        // ASSERT
        let message = format!("{:#}", result.expect_err("no token should unseal"));
        assert!(
            message.contains("TPM2 unseal failed"),
            "message should stay greppable: {message}"
        );
        assert!(
            message.contains("2 tokens tried"),
            "message should name the token count: {message}"
        );
    }

    #[test]
    fn unseal_tokens_rejects_an_empty_set() {
        // ARRANGE
        let tokens: [Tpm2Token; 0] = [];

        // ACT
        let result = unseal_tokens(&tokens, &|_bytes: &[u8]| -> Result<Passphrase> {
            panic!("an empty set must not reach the unseal")
        });

        // ASSERT
        let message = format!("{:#}", result.expect_err("no token should unseal"));
        assert!(
            message.contains("TPM2 unseal failed: no TPM2 token in the LUKS2 header"),
            "message should name the missing token: {message}"
        );
    }

    #[test]
    fn unseal_tokens_reports_a_single_attempted_token() {
        // ARRANGE
        let tokens = [token(1)];
        let unseal = |_bytes: &[u8]| -> Result<Passphrase> { Err(anyhow::anyhow!("stale PCR")) };

        // ACT
        let message = format!(
            "{:#}",
            unseal_tokens(&tokens, &unseal).expect_err("the only token should fail")
        );

        // ASSERT
        assert!(
            message.contains("1 tokens tried"),
            "message should name the token count: {message}"
        );
    }
}
