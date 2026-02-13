//! Update validation and rollback handling after kexec.
//!
//! Handles the automatic validation of system updates after a kexec reboot.
//! Checks if the new kernel booted successfully and runs health checks to
//! determine if the update should be committed or rolled back.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use serde::{Deserialize, Serialize};
use sysconfig::{CONFIG_PATH, HostConfig};

use crate::constants::{SECRETS_DIR, UPDATE_DIR};
use crate::disk;
use crate::uki::{self, Uki};

/// Marker file tracking update validation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationMarker {
    pub update_id: String,
    pub target_image: String,
    pub current_image: String,
    pub timestamp: i64,
}

/// Information about a rolled-back update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackInfo {
    pub update_id: String,
    pub failed_image: String,
    pub reason: String,
    pub rolled_back_at: i64,
}

/// Status of a system update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Unknown,
    Pending,
    Committed,
    RolledBack(String),
}

/// Gets the current status of an update.
pub fn get_update_status(update_id: &str) -> UpdateStatus {
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
        && let Ok(marker) = serde_json::from_str::<ValidationMarker>(&contents)
        && marker.update_id == update_id
    {
        return UpdateStatus::Pending;
    }

    UpdateStatus::Unknown
}

/// Checks for pending validations and handles commit or rollback.
pub fn check_and_handle_pending_validation() -> Result<()> {
    let marker = match load_validation_marker()? {
        Some(m) => m,
        None => return Ok(()),
    };

    kmsg::info!(
        "Found pending validation for update {} -> {}",
        marker.current_image,
        marker.target_image
    );

    if is_old_kernel(&marker) {
        kmsg::info!(
            "Update {} failed - new kernel did not boot successfully",
            &marker.update_id
        );
        rollback_update(&marker, "Kernel failed to boot (kexec failure)")?;
    } else if let Err(e) = health_checks() {
        kmsg::info!("Health checks failed: {}", e);
        rollback_update(&marker, &format!("Health checks failed: {}", e))?;
    } else {
        commit_update(&marker)?;
    }

    Ok(())
}

/// Loads the validation marker from disk if it exists.
fn load_validation_marker() -> Result<Option<ValidationMarker>> {
    let marker_path = Path::new(UPDATE_DIR).join("pending-validation.json");

    if !marker_path.exists() {
        return Ok(None);
    }

    let contents =
        std::fs::read_to_string(&marker_path).context("Failed to read pending-validation.json")?;

    let marker: ValidationMarker =
        serde_json::from_str(&contents).context("Failed to parse pending-validation.json")?;

    Ok(Some(marker))
}

/// Checks if we're running the old kernel (kexec failed).
fn is_old_kernel(marker: &ValidationMarker) -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let expected_marker = format!("muak.update_id={}", marker.update_id);

    !cmdline.contains(&expected_marker)
}

/// Runs health checks to validate the system after update.
fn health_checks() -> Result<()> {
    check_state_partition_writable()?;
    check_network_interfaces()?;
    Ok(())
}

/// Checks if the STATE partition is writable.
fn check_state_partition_writable() -> Result<()> {
    let test_path = "/run/state/.update_health_check";
    std::fs::write(test_path, b"ok").context("STATE partition not writable")?;
    std::fs::remove_file(test_path).context("Failed to clean up health check file")?;
    Ok(())
}

/// Checks if network interfaces are available.
fn check_network_interfaces() -> Result<()> {
    let net_dir =
        std::fs::read_dir("/sys/class/net").context("Failed to read network interfaces")?;

    let non_loopback_count = net_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "lo")
        .count();

    if non_loopback_count == 0 {
        bail!("No non-loopback network interfaces found");
    }

    Ok(())
}

/// Commits the update by installing the new UKI.
fn commit_update(marker: &ValidationMarker) -> Result<()> {
    kmsg::info!(
        "Validation succeeded, committing update {}",
        marker.update_id
    );

    let efi_device = disk::find_partition_by_partname("EFI")
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let mount_point = "/run/mnt/efi";
    disk::mount_efi_partition(&efi_device, mount_point)?;

    let result = install_new_uki_and_finalize(marker, mount_point);

    disk::try_unmount(mount_point);

    result
}

/// Installs the new UKI and updates the config.
fn install_new_uki_and_finalize(marker: &ValidationMarker, mount_point: &str) -> Result<()> {
    std::fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let components = Uki::from_dir(Path::new(UPDATE_DIR));
    let uki_path = uki::get_uki_path(Path::new(mount_point))?;

    let luks_key = read_luks_key_from_cmdline();
    components.build_atomic(&uki_path, luks_key.as_deref())?;

    if sysconfig::system().secureboot {
        let hierarchy = sbolt::keys::load_key_hierarchy(&Path::new(SECRETS_DIR).join("secureboot"))
            .context("Failed to load Secure Boot keys for UKI signing")?;
        Uki::sign(&uki_path, &hierarchy)?;
    }

    update_config_image(&marker.target_image)?;

    cleanup_update_files();

    sync();

    Ok(())
}

/// Updates the system image in the config.
fn update_config_image(new_image: &str) -> Result<()> {
    let contents = std::fs::read_to_string(CONFIG_PATH).context("Failed to read config.toml")?;
    let mut config: HostConfig =
        sysconfig::parse_from_str(&contents).context("Failed to parse config.toml")?;

    config.system.image = new_image.to_string();

    let updated_toml = sysconfig::serialize(&config).context("Failed to serialize config")?;
    std::fs::write(CONFIG_PATH, updated_toml).context("Failed to write updated config.toml")?;

    Ok(())
}

/// Rolls back the update and reboots.
fn rollback_update(marker: &ValidationMarker, reason: &str) -> Result<()> {
    kmsg::info!("Rolling back update {}: {}", marker.update_id, reason);

    save_rollback_info(marker, reason)?;
    cleanup_failed_update();

    sync();

    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;

    unreachable!("If we're here, something went really wrong");
}

/// Saves rollback information to disk.
fn save_rollback_info(marker: &ValidationMarker, reason: &str) -> Result<()> {
    let rollbacks_dir = Path::new("/run/state/rollbacks");
    std::fs::create_dir_all(rollbacks_dir).context("Failed to create rollbacks dir")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let info = RollbackInfo {
        update_id: marker.update_id.clone(),
        failed_image: marker.target_image.clone(),
        reason: reason.to_string(),
        rolled_back_at: now,
    };

    let data = serde_json::to_string_pretty(&info)?;
    let path = rollbacks_dir.join(format!("{}.json", info.update_id));
    std::fs::write(path, data).context("Failed to write rollback info")?;

    Ok(())
}

/// Cleans up update staging files.
fn cleanup_update_files() {
    if let Err(e) = uki::cleanup_dir(Path::new(UPDATE_DIR)) {
        kmsg::warn!("Failed to cleanup update work dir: {}", e);
    }
}

/// Cleans up files from a failed update.
fn cleanup_failed_update() {
    if let Err(e) = uki::cleanup_dir(Path::new(UPDATE_DIR)) {
        kmsg::warn!("Failed to cleanup update work dir during rollback: {}", e);
    }
}

/// Reads the LUKS key from the current `/proc/cmdline`.
fn read_luks_key_from_cmdline() -> Option<Vec<u8>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
    let encoded = cmdline
        .split_whitespace()
        .find(|t| t.starts_with("luks.key="))?
        .strip_prefix("luks.key=")?;
    <base64ct::Base64Unpadded as base64ct::Encoding>::decode_vec(encoded).ok()
}
