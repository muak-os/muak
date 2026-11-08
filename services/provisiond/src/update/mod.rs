//! Update preparation, kexec execution, and post-boot validation.

pub(crate) mod build;
mod commit;
pub(crate) mod document;
mod failure;
pub mod kexec;
mod progress;
mod readiness;
pub(crate) mod rollback;
mod secureboot;
pub(super) mod snapshot;
mod stage;
mod validation;

use std::path::Path;

use anyhow::{Result, bail};
use wizard::cache::Strategy;

use crate::journal::{self, ChangeKind, Entry};
use crate::medium;

/// Staging directory for update operations.
pub(crate) const UPDATE_DIR: &str = "/run/state/update";

/// Base directory for secrets on the mounted STATE partition.
pub(crate) const SECRETS_DIR: &str = "/run/state/secrets";

/// Status of a system update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Unknown,
    Pending,
    Committed,
    RolledBack(String),
}

/// Returns the current status of a given update ID.
pub fn status(update_id: &str) -> UpdateStatus {
    resolve_status(
        update_id,
        journal::find(update_id).as_ref(),
        snapshot::find_pending().is_some_and(|(pending, _)| pending == update_id),
    )
}

pub fn check_and_handle_pending_validation() {
    let Some((update_id, snapshot_path)) = snapshot::find_pending() else {
        return;
    };

    if !has_update_marker() {
        let failed_version = medium::version().unwrap_or_else(|_| "unknown".to_owned());
        match snapshot::restore(
            &update_id,
            &snapshot_path,
            &failed_version,
            "Uncommitted update reverted",
        ) {
            Ok(true) => cleanup_stale(),
            Ok(false) => {
                kmsg::warn!("Revert of {update_id} untracked; snapshot kept for next-boot retry");
            }
            Err(e) => {
                kmsg::warn!("Failed to revert uncommitted update {}: {:#}", update_id, e);
            }
        }
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = validation::validate(&update_id, &snapshot_path).await {
            kmsg::warn!("Pending validation failed: {:#}", e);
        }
    });
}

/// Validates and returns the requested update target release.
///
/// # Errors
///
/// Returns an error when `requested` is empty or when it is a downgrade of `current`.
pub fn resolve_target(current: &str, requested: &str) -> Result<String> {
    if requested.is_empty() {
        bail!("update target version must be specified");
    }

    config::version::check_no_downgrade(requested, current)?;

    Ok(requested.to_owned())
}

/// Returns `true` when the requested update would leave the system unchanged.
pub(crate) fn already_current(
    current_version: &str,
    target_version: &str,
    current_config: &[u8],
    new_config: Option<&[u8]>,
    force: bool,
) -> bool {
    if force || target_version != current_version {
        return false;
    }

    new_config.is_none_or(|bytes| bytes == current_config)
}

/// Maps the offline flag to the pull cache access strategy.
pub(crate) const fn strategy(offline: bool) -> Strategy {
    if offline {
        Strategy::CacheOnly
    } else {
        Strategy::Network
    }
}

pub fn signal_cli_contact() {
    validation::signal_cli_contact();
}

fn resolve_status(update_id: &str, entry: Option<&Entry>, pending: bool) -> UpdateStatus {
    if let Some(entry) = entry
        && entry.kind() == &ChangeKind::Rollback
    {
        return UpdateStatus::RolledBack(entry.reason().unwrap_or("Unknown error").to_owned());
    }

    if pending {
        return UpdateStatus::Pending;
    }

    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains(&format!("muak.update_id={update_id}")) {
        return UpdateStatus::Committed;
    }

    UpdateStatus::Unknown
}

fn has_update_marker() -> bool {
    match std::fs::read_to_string("/proc/cmdline") {
        Ok(cmdline) => cmdline.contains("muak.update_id="),
        Err(error) => {
            kmsg::warn!("Failed to read /proc/cmdline: {error}; assuming an update trial");
            true
        }
    }
}

fn cleanup_stale() {
    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup stale update dir: {e}");
    }
}

#[cfg(test)]
mod tests {
    use wizard::cache::Strategy;

    use super::{UpdateStatus, already_current, resolve_status, resolve_target, strategy};
    use crate::journal::{ChangeKind, Entry};

    const INSTALLED: &str = "host = { name = \"alpha\" }\n";

    #[test]
    fn offline_flag_selects_the_cache_only_strategy() {
        // ARRANGE
        let offline = true;
        let online = false;

        // ACT
        let selected = strategy(offline);
        let default = strategy(online);

        // ASSERT
        assert_eq!(selected, Strategy::CacheOnly);
        assert_eq!(default, Strategy::Network);
    }

    #[test]
    fn requested_release_is_guarded_and_returned() {
        // ARRANGE
        let current = "v1.2.0";

        // ACT
        let newer = resolve_target(current, "v1.3.0");
        let same = resolve_target(current, "v1.2.0");
        let older = resolve_target(current, "v1.1.0");
        let invalid = resolve_target(current, "garbage");

        // ASSERT
        newer.expect("newer release is valid");
        assert_eq!(same.expect("same release is valid"), "v1.2.0");
        older.expect_err("downgrade must fail");
        invalid.expect_err("invalid version must fail");
    }

    #[test]
    fn empty_release_is_rejected() {
        // ARRANGE
        let current = "v1.2.0";

        // ACT
        let error = resolve_target(current, "").expect_err("empty version must fail");

        // ASSERT
        assert!(error.to_string().contains("must be specified"));
    }

    #[test]
    fn already_current_accepts_unchanged_target() {
        // ARRANGE
        let current = INSTALLED.as_bytes();

        // ACT
        let result = already_current("v1.0.0", "v1.0.0", current, None, false);

        // ASSERT
        assert!(result);
    }

    #[test]
    fn already_current_accepts_identical_requested_config() {
        // ARRANGE
        let current = INSTALLED.as_bytes();

        // ACT
        let result = already_current("v1.0.0", "v1.0.0", current, Some(current), false);

        // ASSERT
        assert!(result);
    }

    #[test]
    fn already_current_rejects_different_config() {
        // ARRANGE
        let current = INSTALLED.as_bytes();
        let requested = b"host = { name = \"beta\" }\n";

        // ACT
        let result = already_current("v1.0.0", "v1.0.0", current, Some(requested), false);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn already_current_rejects_different_version() {
        // ARRANGE
        let current = INSTALLED.as_bytes();

        // ACT
        let result = already_current("v1.0.0", "v1.1.0", current, None, false);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn already_current_rejects_forced_request() {
        // ARRANGE
        let current = INSTALLED.as_bytes();

        // ACT
        let result = already_current("v1.0.0", "v1.0.0", current, None, true);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn prepare_failed_entry_keeps_unknown_status() {
        // ARRANGE
        let update_id = "update-1700000001";
        let entry =
            Entry::new(update_id, "system", ChangeKind::PrepareFailed).failed("v2", "fetch failed");

        // ACT
        let status = resolve_status(update_id, Some(&entry), false);

        // ASSERT
        assert_eq!(status, UpdateStatus::Unknown);
    }
}
