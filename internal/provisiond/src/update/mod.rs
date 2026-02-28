//! Update preparation, kexec execution, and post-boot validation.

mod commit;
pub mod kexec;
pub mod marker;
mod rollback;
mod validation;

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rollback::RollbackInfo;
use rustix::fs::sync;
use tokio::sync::mpsc;

use crate::constants::UPDATE_DIR;
use crate::services::proto::provision::PrepareUpdateProgress;
use crate::streaming;
use crate::uki::{self, Uki};

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
    let rollback_path = Path::new("/run/state/rollbacks").join(format!("{}.json", update_id));
    if rollback_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&rollback_path)
            && let Ok(info) = serde_json::from_str::<RollbackInfo>(&contents)
        {
            return UpdateStatus::RolledBack(info.reason);
        }
        return UpdateStatus::RolledBack("Unknown error".to_string());
    }

    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains(&format!("muak.update_id={}", update_id)) {
        return UpdateStatus::Committed;
    }

    let marker_path = Path::new(UPDATE_DIR).join("pending-validation.json");
    if let Ok(contents) = std::fs::read_to_string(&marker_path)
        && let Ok(m) = serde_json::from_str::<marker::ValidationMarker>(&contents)
        && m.update_id == update_id
    {
        return UpdateStatus::Pending;
    }

    UpdateStatus::Unknown
}

/// Prepares an update by staging the UKI components.
pub async fn prepare(
    image: &str,
    extensions: &[String],
    progress: mpsc::Sender<PrepareUpdateProgress>,
) -> Result<String> {
    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: format!("Pulling update image: {}", image),
            ..Default::default()
        },
    )
    .await;

    let staging_dir = create_staging_dir()?;
    Uki::prepare(image, extensions, &staging_dir).await?;

    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: "Finalizing update".to_string(),
            ..Default::default()
        },
    )
    .await;

    let m = marker::create(image)?;
    marker::save(&staging_dir, &m)?;
    sync();

    Ok(m.update_id)
}

/// Checks for a pending validation marker and commits or rolls back.
pub async fn check_and_handle_pending_validation() -> Result<()> {
    let m = match marker::load()? {
        Some(m) => m,
        None => return Ok(()),
    };

    validation::check_pending(&m).await
}

/// Creates the staging directory for update files.
fn create_staging_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&dir).context("Failed to create update staging dir")?;
    Ok(dir)
}

/// Removes the update staging directory.
pub fn cleanup() {
    if let Err(e) = uki::cleanup_dir(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup update work dir: {}", e);
    }
}
