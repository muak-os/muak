//! Update preparation, kexec execution, and post-boot validation.

mod commit;
pub mod kexec;
mod rollback;
pub(super) mod snapshot;
mod validation;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rollback::{ROLLBACKS_DIR, RollbackInfo};
use rustix::fs::sync;
use sysconfig::{CONFIG_PATH, HostConfig};
use tokio::sync::mpsc;

use crate::constants::UPDATE_DIR;
use crate::services::proto::provision::PrepareUpdateProgress;
use crate::streaming;
use crate::uki::Uki;

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
    let rollback_path = Path::new(ROLLBACKS_DIR).join(format!("{}.json", update_id));
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

    if snapshot::path(update_id).exists() {
        return UpdateStatus::Pending;
    }

    UpdateStatus::Unknown
}

/// Prepares an update by staging the UKI components.
pub async fn prepare(
    image: &str,
    extensions: &[String],
    new_config: Option<HostConfig>,
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

    if let Some(ref cfg) = new_config {
        if cfg.system.secureboot
            && !sysconfig::system().secureboot
            && !sbolt::efi::get_setup_mode().unwrap_or(false)
        {
            bail!(
                "Firmware is not in Setup Mode, cannot enable Secure Boot via update. \
                 Please reboot and reset your firmware to Setup Mode and try again."
            );
        }
    }

    Uki::prepare(image, extensions, &staging_dir).await?;

    streaming::send_progress(
        &progress,
        PrepareUpdateProgress {
            message: "Finalizing update".to_string(),
            ..Default::default()
        },
    )
    .await;

    let update_id = snapshot::create(&staging_dir)?;

    if let Some(cfg) = new_config {
        update_config(&cfg)?;
    } else {
        update_config_image(image)?;
    }

    sync();

    Ok(update_id)
}

/// Checks for a pending update snapshot and commits or rolls back.
pub async fn check_and_handle_pending_validation() -> Result<()> {
    let Some((update_id, snapshot_path)) = snapshot::find_pending()? else {
        return Ok(());
    };

    validation::check_pending(&update_id, &snapshot_path).await
}

/// Creates the staging directory for update files.
fn create_staging_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&dir).context("Failed to create update staging dir")?;
    Ok(dir)
}

/// Updates the host system config with a new image, preserving all other fields.
pub(super) fn update_config_image(image: &str) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let mut config: HostConfig =
        sysconfig::parse_from_str(&contents).context("Failed to parse config")?;

    config.system.image = image.to_string();

    let updated_config = sysconfig::serialize(&config).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, updated_config).context("Failed to write updated config")?;

    Ok(())
}

/// Writes all mutable fields from `new_config` into the existing config on-disk.
pub(super) fn update_config(new_config: &HostConfig) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let config: HostConfig =
        sysconfig::parse_from_str(&contents).context("Failed to parse config")?;

    let mut merged = new_config.clone();
    merged.system.disk = config.system.disk.clone();

    let updated_config = sysconfig::serialize(&merged).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, updated_config).context("Failed to write updated config")?;

    Ok(())
}
