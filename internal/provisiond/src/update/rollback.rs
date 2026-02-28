//! Rollback execution and persistence after a failed update.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use serde::{Deserialize, Serialize};

use super::marker::ValidationMarker;

/// Information about a rolled-back update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackInfo {
    pub update_id: String,
    pub failed_image: String,
    pub reason: String,
    pub rolled_back_at: i64,
}

/// Saves rollback info, cleans up staging files, and reboots into the old kernel.
pub fn rollback(m: &ValidationMarker, reason: &str) -> Result<()> {
    println!("Rolling back update {}: {}", m.update_id, reason);
    save(m, reason)?;
    super::cleanup();
    sync();
    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;
    unreachable!("If we're here, something went really wrong")
}

/// Persists rollback information to `/run/state/rollbacks/<update_id>.json`.
fn save(m: &ValidationMarker, reason: &str) -> Result<()> {
    let rollbacks_dir = Path::new("/run/state/rollbacks");
    std::fs::create_dir_all(rollbacks_dir).context("Failed to create rollbacks dir")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let info = RollbackInfo {
        update_id: m.update_id.clone(),
        failed_image: m.target_image.clone(),
        reason: reason.to_string(),
        rolled_back_at: now,
    };

    let data = serde_json::to_string_pretty(&info)?;
    let path = rollbacks_dir.join(format!("{}.json", info.update_id));
    std::fs::write(path, data).context("Failed to write rollback info")?;

    Ok(())
}
