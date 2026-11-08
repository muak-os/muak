//! Stages the update payload: the swap config files and the staged UKI digest.

use core::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use config::system::{self, CONFIG_PATH};
use sha2::{Digest as _, Sha256};

use super::snapshot;
use crate::journal;

const DIGEST_NAME: &str = "uki.sha256";
const READ_BUFFER_LEN: usize = 8192;

/// Writes the update config, keeping the current config as `config.prev` first.
///
/// # Errors
///
/// Returns an error when the current config cannot be read, the new document is
/// not UTF-8, or either file cannot be written.
pub(super) fn config(
    update_id: &str,
    version: &str,
    new_config: Option<Vec<u8>>,
    author: &str,
) -> Result<()> {
    let current = std::fs::read(CONFIG_PATH).context("Failed to read current config")?;
    system::write_atomic(Path::new(snapshot::CONFIG_PREV_PATH), &current)
        .context("Failed to keep the previous config")?;

    let staged = match new_config {
        Some(bytes) => String::from_utf8(bytes).context("Staged config is not valid UTF-8")?,
        None => bumped(&current, version)?,
    };

    let entry = journal::Entry::new(update_id, author, journal::ChangeKind::Update);
    journal::append(&entry, &staged).context("Failed to append config journal entry")?;

    system::write_atomic(Path::new(CONFIG_PATH), staged.as_bytes())
        .context("Failed to write the updated config")
}

/// Writes the lowercase hex SHA-256 of the staged UKI next to the update assets.
///
/// # Errors
///
/// Returns an error when the UKI cannot be read or the digest cannot be written.
pub(super) fn uki_digest(staging_dir: &Path, uki_path: &Path) -> Result<()> {
    let hex = hex_digest(uki_path)?;
    system::write_atomic(&staging_dir.join(DIGEST_NAME), hex.as_bytes())
        .context("Failed to write the staged UKI digest")
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
        write!(hex, "{byte:02x}").context("Failed to encode the UKI digest")?;
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
        let path = dir.path().join("uki.efi");
        fs::write(&path, b"abc").expect("write UKI");

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
        let error = hex_digest(&path).expect_err("missing UKI must fail");

        // ASSERT
        assert!(error.to_string().contains("absent.efi"), "got {error}");
    }
}
