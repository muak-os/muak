//! LUKS key resolution.

use anyhow::{Context as _, Result, bail};
use blockdev::discover;
use disk::role::Role;
use luks2::key::Passphrase;
use rustix::fs::{AtFlags, CWD, Mode, mkdirat, unlinkat};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use tpm2::{blob, device, operations};

const ESP_MOUNT: &str = "/run/efi";
const ESP_KEY_NAME: &str = "luks";
const ESP_FSTYPE: &str = "vfat";

// Resolve the LUKS key for the given device, trying TPM2 unseal, then the ESP key file.
pub(super) fn resolve_key(device: &str) -> Result<Option<Passphrase>> {
    match try_tpm2_unseal(device) {
        Ok(Some(key)) => Ok(Some(key)),
        Ok(None) => Ok(try_esp_key()),
        Err(error) => Err(anyhow::anyhow!("TPM2 unseal failed: {error}")),
    }
}

fn try_tpm2_unseal(device: &str) -> Result<Option<Passphrase>> {
    if !device::is_available(None) {
        return Ok(None);
    }

    let Ok(token) = luks2::read_tpm2_token(device) else {
        return Ok(None);
    };

    let blob_bytes = token
        .sealed_blob()
        .context("Failed to decode TPM2 blob from LUKS token")?;
    let blob = blob::Sealed::deserialize(&blob_bytes).context("Failed to deserialize TPM2 blob")?;

    match operations::unseal(&blob) {
        Ok(key) => {
            kmsg::info!("LUKS key unsealed from TPM2");
            Ok(Some(key))
        }
        Err(error) => {
            kmsg::error!("TPM2 unseal failed: {}", error);
            Err(anyhow::anyhow!("TPM2 unseal failed: {error}"))
        }
    }
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

    decode_key_file(bytes)
}

fn decode_key_file(bytes: Vec<u8>) -> Result<Passphrase> {
    if bytes.len() != luks2::key::VOLUME_KEY_SIZE {
        bail!(
            "ESP LUKS key file has {} bytes, expected {}",
            bytes.len(),
            luks2::key::VOLUME_KEY_SIZE
        );
    }

    Ok(Passphrase::new(bytes))
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

    #[test]
    fn decode_key_file_accepts_key_of_expected_length() {
        // ARRANGE
        let bytes = vec![7_u8; luks2::key::VOLUME_KEY_SIZE];

        // ACT
        let result = decode_key_file(bytes);

        // ASSERT
        assert_eq!(
            result.as_ref().map(|value| value.as_slice()).ok(),
            Some([7_u8; luks2::key::VOLUME_KEY_SIZE].as_ref())
        );
    }

    #[test]
    fn decode_key_file_rejects_empty_key() {
        // ARRANGE
        let bytes = Vec::new();

        // ACT
        let result = decode_key_file(bytes);

        // ASSERT
        assert!(result.unwrap_err().to_string().contains("expected 64"));
    }
}
