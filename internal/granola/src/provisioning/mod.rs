mod install;
mod uki;
mod update;
mod validation;

pub use update::update;
pub use validation::{UpdateStatus, check_and_handle_pending_validation, get_update_status};

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use nix::mount::{MsFlags, mount, umount};
use serde::{Deserialize, Serialize};

use crate::config::HostConfig;
use crate::disk;
use uki::{Uki, UkiConfig};

pub(crate) const INSTALL_DIR: &str = "/run/install";
pub(crate) const UPDATE_DIR: &str = "/run/state/update";
pub(crate) const DEFAULT_CMDLINE: &str = "console=tty0 console=ttyS0 init=/init";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationStatus {
    Live,
    Installed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationMarker {
    pub update_id: String,
    pub target_image: String,
    pub current_image: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackInfo {
    pub update_id: String,
    pub failed_image: String,
    pub reason: String,
    pub rolled_back_at: i64,
}

pub fn status() -> InstallationStatus {
    if disk::find_partition_by_partname("STATE").is_some() {
        InstallationStatus::Installed
    } else {
        InstallationStatus::Live
    }
}

pub async fn install(force: bool, config: HostConfig) -> Result<()> {
    let disk_path = config.system.disk.clone();

    tokio::task::spawn_blocking(move || install::install(&disk_path, force, &config))
        .await
        .context("Install task panicked")?
}

pub async fn prepare_update(image: &str, extensions: &[String]) -> Result<String> {
    let image = image.to_string();
    let extensions = extensions.to_vec();

    tokio::task::spawn_blocking(move || update::prepare(&image, &extensions))
        .await
        .context("Prepare update task panicked")?
}

pub(crate) fn prepare_uki(
    installer_image: &str,
    extensions: &[String],
    work_dir: &Path,
) -> Result<Uki> {
    let config = UkiConfig {
        installer_image,
        extensions,
        work_dir,
        cmdline: DEFAULT_CMDLINE,
    };

    uki::prepare_uki_components(&config)
}

pub(crate) fn mount_efi_partition(efi_device: &str, mount_point: &str) -> Result<()> {
    kmsg::info!(
        @ "provisioning",
        "Mounting EFI partition {} at {}",
        efi_device,
        mount_point
    );

    fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;

    mount(
        Some(efi_device),
        mount_point,
        Some("vfat"),
        MsFlags::MS_NOATIME,
        None::<&str>,
    )
    .with_context(|| {
        format!(
            "Failed to mount EFI partition {} at {}",
            efi_device, mount_point
        )
    })?;

    Ok(())
}

pub(crate) fn unmount_partition(mount_point: &str) {
    if let Err(e) = umount(mount_point) {
        kmsg::warn!(
            @ "provisioning",
            "Failed to unmount {}: {}",
            mount_point,
            e
        );
    }
}
