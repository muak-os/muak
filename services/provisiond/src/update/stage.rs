//! Stages the update payload: the swap config files and the current ESP boot hash.

use core::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use ::disk::role::Role;
use anyhow::{Context as _, Result};
use config::system::{self, CONFIG_PATH};
use sha2::{Digest as _, Sha256};

use super::snapshot;
use crate::disk::roles;
use crate::{efi, journal};

const ESP_HASH_NAME: &str = "esp.sha256";
const READ_BUFFER_LEN: usize = 8192;

/// Writes the update config, keeping the current config as `config.prev` first.
///
/// # Errors
///
/// Returns an error when the current config cannot be read, the new document is
/// rejected for an update, or either file cannot be written.
pub(super) fn config(
    update_id: &str,
    version: &str,
    new_config: Option<Vec<u8>>,
    author: &str,
) -> Result<()> {
    let current = std::fs::read(CONFIG_PATH).context("Failed to read current config")?;
    let staged = match new_config {
        Some(bytes) => String::from_utf8(bytes).context("Staged config is not valid UTF-8")?,
        None => bumped(&current, version)?,
    };
    check_against_installed(&staged)?;

    system::write_atomic(Path::new(snapshot::CONFIG_PREV_PATH), &current)
        .context("Failed to keep the previous config")?;
    system::write_atomic(Path::new(CONFIG_PATH), staged.as_bytes())
        .context("Failed to write the updated config")?;

    let entry = journal::Entry::new(update_id, author, journal::ChangeKind::Update);
    journal::append(&entry, &staged).context("Failed to append config journal entry")?;

    system::write_atomic(Path::new(snapshot::PENDING_ID_PATH), update_id.as_bytes())
        .context("Failed to record the update id")
}

/// Records the lowercase hex SHA-256 of the boot file currently on the ESP.
pub(super) fn esp_hash(staging_dir: &Path) {
    let Some(device) = roles::device(Role::Esp) else {
        kmsg::warn!("ESP partition not found; skipping the ESP boot file hash");
        return;
    };

    if let Err(error) = record_esp_hash(staging_dir, &device) {
        kmsg::warn!("Failed to record the ESP boot file hash: {error:#}");
    }
}

fn check_against_installed(staged: &str) -> Result<()> {
    let Ok(candidate) = system::parse_from_str(staged) else {
        kmsg::warn!("Staged config does not decode with the installed schema; storing it as is");
        return Ok(());
    };
    let installed = system::config()?;

    candidate
        .validate_for_update(installed)
        .context("Staged config is not acceptable for an update")
}

fn record_esp_hash(staging_dir: &Path, device: &str) -> Result<()> {
    efi::mount(device)?;
    let path = Path::new(efi::MOUNT_POINT).join(esp::arch::Arch::current().boot_path());
    let hashed = hex_digest(&path);
    efi::unmount();

    let hex = hashed?;
    system::write_atomic(&staging_dir.join(ESP_HASH_NAME), hex.as_bytes())
        .context("Failed to write the ESP boot file hash")
}

fn bumped(current: &[u8], version: &str) -> Result<String> {
    let text = core::str::from_utf8(current).context("Installed config is not valid UTF-8")?;
    let mut config: system::Config =
        system::parse_from_str(text).context("Failed to parse the installed config")?;
    version.clone_into(&mut config.host.version);

    system::serialize(&config).context("Failed to serialize the updated config")
}

fn hex_digest(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut file =
        File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut buffer = [0_u8; READ_BUFFER_LEN];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        let chunk = buffer
            .get(..read)
            .context("Failed to read a whole buffer chunk")?;
        hasher.update(chunk);
    }

    let digest = hasher.finalize();
    let mut hex = String::new();
    for byte in &digest {
        write!(hex, "{byte:02x}").context("Failed to encode the boot file digest")?;
    }

    Ok(hex)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::hex_digest;

    #[test]
    fn digest_is_lowercase_hex_sha256() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("BOOTX64.EFI");
        fs::write(&path, b"abc").expect("write boot file");

        // ACT
        let hex = hex_digest(&path).expect("digest a readable file");

        // ASSERT
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn missing_file_is_rejected() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("absent.efi");

        // ACT
        let error = hex_digest(&path).expect_err("missing boot file must fail");

        // ASSERT
        assert!(error.to_string().contains("absent.efi"), "got {error}");
    }
}
