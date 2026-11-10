//! Stages the update payload: the swap config files.

use std::path::Path;

use anyhow::{Context as _, Result};
use config::system::{self, CONFIG_PATH};

use super::snapshot;
use crate::journal;

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

fn bumped(current: &[u8], version: &str) -> Result<String> {
    let text = core::str::from_utf8(current).context("Installed config is not valid UTF-8")?;
    let mut config: system::Config =
        system::parse_from_str(text).context("Failed to parse the installed config")?;
    version.clone_into(&mut config.host.version);

    system::serialize(&config).context("Failed to serialize the updated config")
}
