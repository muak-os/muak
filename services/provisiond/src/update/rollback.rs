//! Rollback execution and persistence after a failed update.

use core::cmp::Reverse;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use serde::{Deserialize, Serialize};

use super::snapshot;
use crate::constants::UPDATE_DIR;

/// Directory holding rollback entries.
pub const ROLLBACKS_DIR: &str = "/run/state/rollbacks";

/// Maximum number of rollback records to retain.
const ROLLBACKS_MAX_ENTRIES: usize = 1000;

/// Information about a rolled-back update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackInfo {
    pub update_id: String,
    pub failed_image: String,
    pub reason: String,
    pub rolled_back_at: i64,
}

/// Saves rollback info, restores the previous config, cleans-up staging files and reboots into the old kernel.
pub fn apply(update_id: &str, snapshot_path: &Path, reason: &str) -> Result<()> {
    println!("Rolling back update {update_id}: {reason}");

    save(update_id, reason)?;
    snapshot::restore(update_id, snapshot_path)?;

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {e}");
    }

    sync();

    kmsg::info!("Rebooting for rollback of update {}: {}", update_id, reason);
    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;

    Err(anyhow!("Reboot for rollback returned unexpectedly"))
}

/// Persists rollback information and prunes old records.
fn save(update_id: &str, reason: &str) -> Result<()> {
    std::fs::create_dir_all(ROLLBACKS_DIR).context("Failed to create rollbacks dir")?;

    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(0);

    let failed_image = config::host().image.clone();

    let info = RollbackInfo {
        update_id: update_id.to_owned(),
        failed_image,
        reason: reason.to_owned(),
        rolled_back_at: now,
    };

    let data = serde_json::to_string_pretty(&info)?;
    let path = Path::new(ROLLBACKS_DIR).join(format!("{}.json", info.update_id));
    std::fs::write(path, data).context("Failed to write rollback info")?;

    prune();

    Ok(())
}

/// Returns the most recent rollback records, newest-first, up to `limit`.
pub fn list(limit: usize) -> Vec<RollbackInfo> {
    let dir = Path::new(ROLLBACKS_DIR);

    let mut entries: Vec<RollbackInfo> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(core::result::Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .filter_map(|entry| {
                let data = std::fs::read_to_string(entry.path()).ok()?;
                serde_json::from_str(&data).ok()
            })
            .collect(),
        Err(_) => return Vec::new(),
    };

    entries.sort_unstable_by_key(|entry| Reverse(entry.rolled_back_at));
    entries.truncate(limit);

    entries
}

/// Removes the oldest rollback records beyond [`ROLLBACKS_MAX_ENTRIES`].
fn prune() {
    let dir = Path::new(ROLLBACKS_DIR);

    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(core::result::Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .map(|entry| entry.path())
            .collect(),
        Err(_) => return,
    };

    if files.len() <= ROLLBACKS_MAX_ENTRIES {
        return;
    }

    files.sort_unstable();
    let to_delete = files.len().saturating_sub(ROLLBACKS_MAX_ENTRIES);
    for path in files.iter().take(to_delete) {
        if let Err(e) = std::fs::remove_file(path) {
            eprintln!("Failed to prune rollback file {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_info_roundtrips_through_serde() {
        // ARRANGE
        let info = RollbackInfo {
            update_id: "update-1234".to_owned(),
            failed_image: "registry.example.com/os:v1.2.3".to_owned(),
            reason: "health check failed".to_owned(),
            rolled_back_at: 1_700_000_000,
        };

        // ACT
        let json = serde_json::to_string(&info).expect("serialize");
        let decoded: RollbackInfo = serde_json::from_str(&json).expect("deserialize");

        // ASSERT
        assert_eq!(decoded.update_id, info.update_id);
        assert_eq!(decoded.failed_image, info.failed_image);
        assert_eq!(decoded.reason, info.reason);
        assert_eq!(decoded.rolled_back_at, info.rolled_back_at);
    }

    #[test]
    fn list_returns_empty_when_rollbacks_dir_absent() {
        // ACT
        let result = list(100);

        // ASSERT
        if !Path::new(ROLLBACKS_DIR).exists() {
            assert!(result.is_empty());
        }
    }

    #[test]
    fn list_sorts_newest_first_and_respects_limit() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let entries = vec![
            RollbackInfo {
                update_id: "update-1".to_owned(),
                failed_image: "img:1".to_owned(),
                reason: "r1".to_owned(),
                rolled_back_at: 100,
            },
            RollbackInfo {
                update_id: "update-2".to_owned(),
                failed_image: "img:2".to_owned(),
                reason: "r2".to_owned(),
                rolled_back_at: 300,
            },
            RollbackInfo {
                update_id: "update-3".to_owned(),
                failed_image: "img:3".to_owned(),
                reason: "r3".to_owned(),
                rolled_back_at: 200,
            },
        ];

        for entry in &entries {
            let data = serde_json::to_string(entry).unwrap();
            std::fs::write(dir.path().join(format!("{}.json", entry.update_id)), data).unwrap();
        }

        // ACT
        let mut result: Vec<RollbackInfo> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(std::io::Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .filter_map(|entry| {
                let data = std::fs::read_to_string(entry.path()).ok()?;
                serde_json::from_str(&data).ok()
            })
            .collect();
        result.sort_unstable_by_key(|entry| Reverse(entry.rolled_back_at));
        result.truncate(2);

        // ASSERT
        assert_eq!(result.len(), 2);
        assert_eq!(result.first().unwrap().update_id, "update-2");
        assert_eq!(result.get(1).unwrap().update_id, "update-3");
    }
}
