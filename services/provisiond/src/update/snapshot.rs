//! Previous config life cycle: keep, locate, read, and restore the swap copy.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use anyhow::{Context as _, Result};
use config::system::CONFIG_PATH;

use crate::journal::{self, Entry};

/// Path of the config kept from before the staged update.
pub(crate) const CONFIG_PREV_PATH: &str = "/run/state/config.prev";

/// Path of the update id recorded by the last prepare.
pub(crate) const PENDING_ID_PATH: &str = "/run/state/update/id";

/// Synthetic update ID used when the boot cmdline carries no update marker.
pub(crate) const UNKNOWN_ID: &str = "update-unknown";

const CMDLINE: &str = "/proc/cmdline";
const UPDATE_ID_PREFIX: &str = "muak.update_id=";

/// Returns the pending `(update_id, path)` of a kept previous config.
pub fn find_pending() -> Option<(String, PathBuf)> {
    let path = PathBuf::from(CONFIG_PREV_PATH);
    if !path.exists() {
        return None;
    }

    let cmdline = fs::read_to_string(CMDLINE).unwrap_or_default();
    let recorded = fs::read_to_string(PENDING_ID_PATH).ok();

    Some((pending_id(&cmdline, recorded.as_deref()), path))
}

/// Returns the path to the previous config kept by a prepare.
pub fn path() -> PathBuf {
    PathBuf::from(CONFIG_PREV_PATH)
}

/// Lifts `host.version` from a config document without validating it.
pub(crate) fn host_version(raw: &str) -> Option<String> {
    let parsed = document(raw)?;
    let host = parsed.get("host")?;

    host.get("version")?.as_str().map(str::to_owned)
}

/// Lifts `host.secureboot` from a TOML document, defaulting to `false` when absent.
pub(crate) fn host_secureboot(raw: &str) -> Option<bool> {
    let parsed = document(raw)?;

    Some(
        parsed
            .get("host")
            .and_then(|host| host.get("secureboot"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    )
}

/// Restores the previous config by rename and records the rollback, returning journal success.
pub fn restore(
    update_id: &str,
    prev_path: &Path,
    failed_version: &str,
    reason: &str,
) -> Result<bool> {
    let contents = match fs::read_to_string(prev_path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            kmsg::warn!("No previous config to restore for {update_id}");
            return Ok(true);
        }
        Err(e) => return Err(e).context("Failed to read the previous config"),
    };

    let entry = Entry::new(update_id, "system", journal::ChangeKind::Rollback)
        .failed(failed_version, reason);
    let tracked = match journal::append(&entry, &contents) {
        Ok(()) => true,
        Err(e) => {
            kmsg::warn!("Rollback of {update_id} is untracked (journal write failed: {e})");
            false
        }
    };

    match fs::rename(prev_path, Path::new(CONFIG_PATH)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            kmsg::warn!("No previous config to restore for {update_id}");
        }
        Err(e) => return Err(e).context("Failed to restore config from the previous one"),
    }

    Ok(tracked)
}

/// Generates a unique update ID from the current wall clock.
pub fn generate_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    format!("update-{timestamp}")
}

fn document(raw: &str) -> Option<toml::Value> {
    toml::from_str(raw).ok()
}

fn pending_id(cmdline: &str, recorded: Option<&str>) -> String {
    if cmdline.contains(UPDATE_ID_PREFIX) {
        return update_id(cmdline);
    }

    recorded
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(|| update_id(cmdline), str::to_owned)
}

fn update_id(cmdline: &str) -> String {
    cmdline
        .split_whitespace()
        .find_map(|arg| arg.strip_prefix(UPDATE_ID_PREFIX))
        .map_or_else(|| UNKNOWN_ID.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{
        UNKNOWN_ID, generate_id, host_secureboot, host_version, path, pending_id, update_id,
    };

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
    fn recorded_id_is_used_without_an_update_marker() {
        // ARRANGE
        let cmdline = "root=/dev/sda1 quiet ro";
        let recorded = Some("update-1700000002\n");

        // ACT
        let id = pending_id(cmdline, recorded);

        // ASSERT
        assert_eq!(id, "update-1700000002");
    }

    #[test]
    fn update_marker_wins_over_the_recorded_id() {
        // ARRANGE
        let cmdline = "root=/dev/sda1 muak.update_id=update-1700000003 ro";
        let recorded = Some("update-1700000002\n");

        // ACT
        let id = pending_id(cmdline, recorded);

        // ASSERT
        assert_eq!(id, "update-1700000003");
    }

    #[test]
    fn missing_recorded_id_falls_back_to_the_synthetic_id() {
        // ARRANGE
        let cmdline = "root=/dev/sda1 quiet ro";
        let recorded = None;

        // ACT
        let id = pending_id(cmdline, recorded);

        // ASSERT
        assert_eq!(id, UNKNOWN_ID);
    }

    #[test]
    fn lifting_ignores_unknown_keys() {
        // ARRANGE
        let raw = r#"
            [host]
            version = "v1.2.0"
            secureboot = true
            unknown_field = "ignored"

            [future_section]
            mode = "turbo"
        "#;

        // ACT
        let version = host_version(raw);
        let secureboot = host_secureboot(raw);

        // ASSERT
        assert_eq!(version.as_deref(), Some("v1.2.0"));
        assert_eq!(secureboot, Some(true));
    }

    #[test]
    fn lifting_tolerates_a_future_api_version() {
        // ARRANGE
        let raw = r#"
            api_version = "muak.dev/config/v9-future"

            [host]
            version = "v2.0.0"
        "#;

        // ACT
        let version = host_version(raw);
        let secureboot = host_secureboot(raw);

        // ASSERT
        assert_eq!(version.as_deref(), Some("v2.0.0"));
        assert_eq!(secureboot, Some(false));
    }

    #[test]
    fn lifting_a_non_toml_document_yields_nothing() {
        // ARRANGE
        let raw = "this is not a toml document";

        // ACT
        let version = host_version(raw);
        let secureboot = host_secureboot(raw);

        // ASSERT
        assert_eq!(version, None);
        assert_eq!(secureboot, None);
    }
}
