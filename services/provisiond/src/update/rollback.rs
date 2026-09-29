//! Rollback execution after a failed update.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use super::UPDATE_DIR;
use super::snapshot;

/// Restores the committed config and reboots into the old kernel.
///
/// # Errors
///
/// Returns an error when the config cannot be restored or the reboot fails.
pub fn apply(update_id: &str, snapshot_path: &Path, reason: &str) -> Result<()> {
    kmsg::info!("Rolling back update {update_id}: {reason}");

    let tracked = snapshot::restore(update_id, snapshot_path, reason)?;
    if tracked {
        if let Err(e) = fs::remove_dir_all(Path::new(UPDATE_DIR)) {
            eprintln!("Failed to cleanup update work dir: {e}");
        }
    } else {
        kmsg::warn!("Rollback of {update_id} untracked. Snapshot kept for next-boot retry");
    }

    sync();

    kmsg::info!("Rebooting for rollback of update {}: {}", update_id, reason);
    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;

    Err(anyhow::anyhow!("Reboot for rollback returned unexpectedly"))
}
