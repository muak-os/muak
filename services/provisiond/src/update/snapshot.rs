//! Previous config life cycle: keep, locate, read, and restore the swap copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use config::system::CONFIG_PATH;

use crate::journal::{self, Entry};

/// Path of the config kept from before the staged update.
pub(crate) const CONFIG_PREV_PATH: &str = "/run/state/config.prev";

/// Synthetic update ID used when the boot cmdline carries no update marker.
pub(crate) const UNKNOWN_ID: &str = "update-unknown";

/// Returns the pending `(update_id, path)` of a kept previous config.
pub fn find_pending() -> Option<(String, PathBuf)> {
    let path = PathBuf::from(CONFIG_PREV_PATH);
    if !path.exists() {
        return None;
    }

    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();

    Some((update_id(&cmdline), path))
}

/// Returns the path to the previous config kept by a prepare.
pub fn path() -> PathBuf {
    PathBuf::from(CONFIG_PREV_PATH)
}

/// Reads and parses the kept previous config.
pub fn read_config(prev_path: &Path) -> Result<config::system::Config> {
    let contents = fs::read_to_string(prev_path).context("Failed to read the previous config")?;

    config::system::parse_from_str(&contents).context("Failed to parse the previous config")
}

/// Restores the system config from the kept previous config, overwriting the
/// current, and records the rollback. Returns `false` when the journal write failed.
pub fn restore(update_id: &str, prev_path: &Path, reason: &str) -> Result<bool> {
    let contents = fs::read_to_string(prev_path).context("Failed to read the previous config")?;
    config::system::write_atomic(Path::new(CONFIG_PATH), contents.as_bytes())
        .context("Failed to restore config from the previous one")?;

    let failed_version = config::system::host()?.version.clone();
    let entry = Entry::new(update_id, "system", journal::ChangeKind::Rollback)
        .failed(&failed_version, reason);

    match journal::append(&entry, &contents) {
        Ok(()) => Ok(true),
        Err(e) => {
            kmsg::warn!("Rollback of {update_id} is untracked (journal write failed: {e})");
            Ok(false)
        }
    }
}

/// Generates a unique update ID from the current wall clock.
pub fn generate_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    format!("update-{timestamp}")
}

fn update_id(cmdline: &str) -> String {
    cmdline
        .split_whitespace()
        .find_map(|arg| arg.strip_prefix("muak.update_id="))
        .map_or_else(|| UNKNOWN_ID.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{UNKNOWN_ID, generate_id, path, read_config, update_id};

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
    fn path_points_at_the_kept_previous_config() {
        // ACT
        let snapshot_path = path();

        // ASSERT
        assert_eq!(
            snapshot_path,
            std::path::Path::new("/run/state/config.prev")
        );
    }

    #[test]
    fn update_id_is_read_from_the_boot_cmdline() {
        // ARRANGE
        let cmdline = "root=/dev/sda1 quiet muak.update_id=update-1700000001 ro";

        // ACT
        let id = update_id(cmdline);

        // ASSERT
        assert_eq!(id, "update-1700000001");
    }

    #[test]
    fn missing_update_marker_yields_the_synthetic_id() {
        // ARRANGE
        let cmdline = "root=/dev/sda1 quiet ro";

        // ACT
        let id = update_id(cmdline);

        // ASSERT
        assert_eq!(id, UNKNOWN_ID);
    }

    #[test]
    fn read_config_parses_snapshot() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir");
        let snapshot_path = dir.path().join("config.prev");
        let mut config = config::system::Config::default();
        config.host.version = "v1.2.3".to_owned();
        let contents = config::system::serialize(&config).expect("serialize config");
        fs::write(&snapshot_path, contents).expect("write snapshot");

        // ACT
        let parsed = read_config(&snapshot_path).expect("read config snapshot");

        // ASSERT
        assert_eq!(parsed.host.version, "v1.2.3");
    }
}
