//! Rollback execution and persistence after a failed update.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use serde::{Deserialize, Serialize};

use super::snapshot;
use crate::constants::UPDATE_DIR;

pub const ROLLBACKS_DIR: &str = "/run/state/rollbacks";

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
    println!("Rolling back update {}: {}", update_id, reason);

    save(update_id, reason)?;
    snapshot::restore(snapshot_path)?;

    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }

    sync();

    kmsg::info!("Rebooting for rollback of update {}: {}", update_id, reason);
    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;
    unreachable!("If we're here, something went really wrong")
}

/// Persists rollback information.
fn save(update_id: &str, reason: &str) -> Result<()> {
    std::fs::create_dir_all(ROLLBACKS_DIR).context("Failed to create rollbacks dir")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let failed_image = sysconfig::system().image.clone();

    let info = RollbackInfo {
        update_id: update_id.to_string(),
        failed_image,
        reason: reason.to_string(),
        rolled_back_at: now,
    };

    let data = serde_json::to_string_pretty(&info)?;
    let path = Path::new(ROLLBACKS_DIR).join(format!("{}.json", info.update_id));
    std::fs::write(path, data).context("Failed to write rollback info")?;

    Ok(())
}
