//! Update preparation, kexec execution, and post-boot validation.

mod commit;
pub mod kexec;
pub(crate) mod rollback;
pub(super) mod snapshot;
mod validation;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use config::{CONFIG_PATH, SystemConfig};
use rollback::{ROLLBACKS_DIR, RollbackInfo};
use rustix::fs::sync;
use tokio::sync::mpsc;

use crate::constants::UPDATE_DIR;
use crate::history::{self, ChangeKind};
use crate::ipc::proto::provision::PrepareUpdateProgress;
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

    if snapshot::path(update_id).exists() {
        return UpdateStatus::Pending;
    }

    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains(&format!("muak.update_id={}", update_id)) {
        return UpdateStatus::Committed;
    }

    UpdateStatus::Unknown
}

/// Prepares an update by staging the UKI components.
pub async fn prepare(
    image: &str,
    extensions: &[String],
    new_config: Option<SystemConfig>,
    author: &str,
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
        let secure_boot_active = sbolt::efi::get_secure_boot().unwrap_or(false);
        let setup_mode = sbolt::efi::get_setup_mode().unwrap_or(false);
        if cfg.host.secureboot && !secure_boot_active && !setup_mode {
            bail!(
                "Firmware is not in Setup Mode, cannot enroll Secure Boot keys. \
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
        update_config(&update_id, &cfg, author)?;
    } else {
        update_config_image(&update_id, image, author)?;
    }

    sync();

    Ok(update_id)
}

/// Checks for a pending update snapshot and spawns validation in the background.
pub fn check_and_handle_pending_validation() -> Result<()> {
    let Some((update_id, snapshot_path)) = snapshot::find_pending()? else {
        return Ok(());
    };

    if !has_update_marker() {
        cleanup_stale();
        return Ok(());
    }

    tokio::spawn(async move {
        if let Err(e) = validation::validate(&update_id, &snapshot_path).await {
            kmsg::warn!("Pending validation failed: {}", e);
        }
    });

    Ok(())
}

/// Returns true if the current boot has the update marker in the cmdline.
fn has_update_marker() -> bool {
    std::fs::read_to_string("/proc/cmdline")
        .unwrap_or_default()
        .contains("muak.update_id=")
}

/// Removes stale update files left from a previous boot cycle.
fn cleanup_stale() {
    if let Err(e) = std::fs::remove_dir_all(Path::new(UPDATE_DIR)) {
        eprintln!("Failed to cleanup stale update dir: {}", e);
    }
}

/// Creates the staging directory for update files.
fn create_staging_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(UPDATE_DIR);
    fs::create_dir_all(&dir).context("Failed to create update staging dir")?;
    Ok(dir)
}

/// Updates the host system config with a new image, preserving all other fields.
pub(super) fn update_config_image(update_id: &str, image: &str, author: &str) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let mut config: SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config")?;

    config.host.image = image.to_string();

    let updated_config = config::serialize(&config).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, &updated_config).context("Failed to write updated config")?;

    if let Err(e) = history::record(update_id, author, ChangeKind::Update, &updated_config) {
        eprintln!("Failed to record config history: {}", e);
    }

    Ok(())
}

/// Writes all mutable fields to the existing config on-disk.
pub(super) fn update_config(
    update_id: &str,
    new_config: &SystemConfig,
    author: &str,
) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config")?;
    let config: SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config")?;

    let mut merged = new_config.clone();
    merged.disk = config.disk.clone();

    let updated_config = config::serialize(&merged).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, &updated_config).context("Failed to write updated config")?;

    if let Err(e) = history::record(update_id, author, ChangeKind::Update, &updated_config) {
        eprintln!("Failed to record config history: {}", e);
    }

    Ok(())
}

/// Signals that the CLI has contacted provisiond during validation.
pub fn signal_cli_contact() {
    validation::signal_cli_contact();
}
