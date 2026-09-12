//! Config snapshot life cycle: create, locate, read, and restore.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use config::{CONFIG_EXTENSION, CONFIG_PATH};

use super::UPDATE_DIR;
use crate::journal::{self, Entry};

/// Generates a unique update ID and saves a copy of the current config to `UPDATE_DIR`.
pub fn create(staging_dir: &Path) -> Result<String> {
    let update_id = generate_id();
    let contents =
        fs::read_to_string(CONFIG_PATH).context("Failed to read current config for snapshot")?;
    fs::write(
        staging_dir.join(format!("{update_id}.{CONFIG_EXTENSION}")),
        contents,
    )
    .context("Failed to write config snapshot")?;

    Ok(update_id)
}

/// Scans `UPDATE_DIR` for a pending snapshot and returns `(update_id, path)` if found.
pub fn find_pending() -> Result<Option<(String, PathBuf)>> {
    let dir = Path::new(UPDATE_DIR);
    if !dir.exists() {
        return Ok(None);
    }

    let entry = fs::read_dir(dir)
        .context("Failed to read update dir")?
        .filter_map(core::result::Result::ok)
        .find(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            name_str.starts_with("update-") && name_str.ends_with(&format!(".{CONFIG_EXTENSION}"))
        });

    let Some(entry) = entry else {
        return Ok(None);
    };

    let path = entry.path();
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .context("Snapshot path has no file stem")?;

    Ok(Some((stem, path)))
}

/// Returns the path to the snapshot file for a given update ID.
pub fn path(update_id: &str) -> PathBuf {
    Path::new(UPDATE_DIR).join(format!("{update_id}.{CONFIG_EXTENSION}"))
}

/// Reads `host.image` from a snapshot file.
pub fn read_image(snapshot_path: &Path) -> Result<String> {
    let contents = fs::read_to_string(snapshot_path).context("Failed to read config snapshot")?;
    let cfg: config::SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config snapshot")?;
    Ok(cfg.host.image)
}

/// Restores the system config from a snapshot file, overwriting the current, and records history.
pub fn restore(update_id: &str, snapshot_path: &Path, reason: &str) -> Result<()> {
    let contents = fs::read_to_string(snapshot_path).context("Failed to read config snapshot")?;
    config::write_atomic(Path::new(CONFIG_PATH), contents.as_bytes())
        .context("Failed to restore config from snapshot")?;

    let failed_image = config::host().image.clone();
    let entry = Entry::new(update_id, "system", journal::ChangeKind::Rollback)
        .rolled_back(&failed_image, reason);
    if let Err(e) = journal::append(&entry, &contents) {
        eprintln!("Failed to append rollback journal entry: {e}");
    }

    Ok(())
}

fn generate_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    format!("update-{timestamp}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_has_expected_prefix() {
        // ACT
        let id = generate_id();

        // ASSERT
        assert!(
            id.starts_with("update-"),
            "id '{id}' must start with 'update-'"
        );
    }

    #[test]
    fn generate_id_suffix_is_numeric() {
        // ACT
        let id = generate_id();

        // ASSERT
        let suffix = id.strip_prefix("update-").expect("prefix present");
        suffix
            .parse::<u64>()
            .expect("suffix must be a valid u64 timestamp");
    }

    #[test]
    fn path_returns_correct_path_for_update_id() {
        // ARRANGE
        let update_id = "update-1700000000";

        // ACT
        let snapshot_path = path(update_id);

        // ASSERT
        let expected = format!("{UPDATE_DIR}/{update_id}.{CONFIG_EXTENSION}");
        assert_eq!(snapshot_path, std::path::Path::new(&expected));
    }
}
