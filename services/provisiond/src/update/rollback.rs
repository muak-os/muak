//! Rollback execution after a failed update.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use super::{UPDATE_DIR, secureboot, snapshot};
use crate::medium;

/// Restores the committed config and reboots into the old kernel.
///
/// # Errors
///
/// Returns an error when the config cannot be restored or the reboot fails.
pub fn apply(update_id: &str, snapshot_path: &Path, reason: &str) -> Result<()> {
    kmsg::info!("Rolling back update {update_id}: {reason}");

    let previous = fs::read_to_string(snapshot_path).ok();
    let secureboot_enabled = previous
        .as_deref()
        .and_then(snapshot::host_secureboot)
        .or_else(|| config::system::host().ok().map(|host| host.secureboot))
        .unwrap_or(false);

    let failed_version = medium::version().unwrap_or_else(|_| "unknown".to_owned());
    let tracked = snapshot::restore(update_id, snapshot_path, &failed_version, reason)?;
    if tracked {
        if let Err(e) = fs::remove_dir_all(Path::new(UPDATE_DIR)) {
            eprintln!("Failed to cleanup update work dir: {e}");
        }
    } else {
        kmsg::warn!("Rollback of {update_id} untracked. Snapshot kept for next-boot retry");
    }

    if secureboot_enabled {
        secureboot::re_enroll_if_missing()?;
    }

    sync();

    kmsg::info!("Rebooting for rollback of update {}: {}", update_id, reason);
    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;

    Err(anyhow::anyhow!("Reboot for rollback returned unexpectedly"))
}
