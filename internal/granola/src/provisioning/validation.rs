use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};
use sysconfig::{CONFIG_PATH, HostConfig};

use super::uki::{self, Uki};
use super::{RollbackInfo, UPDATE_DIR, ValidationMarker, mount_efi_partition, unmount_partition};
use crate::constants::SECRETS_DIR;
use crate::disk;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Unknown,
    Pending,
    Committed,
    RolledBack(String),
}

pub fn get_update_status(update_id: &str) -> UpdateStatus {
    let rollback_path = Path::new("/run/state/rollbacks").join(format!("{}.json", update_id));
    if rollback_path.exists() {
        if let Ok(contents) = fs::read_to_string(&rollback_path)
            && let Ok(info) = serde_json::from_str::<RollbackInfo>(&contents)
        {
            return UpdateStatus::RolledBack(info.reason);
        }
        return UpdateStatus::RolledBack("Unknown error".to_string());
    }

    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    if cmdline.contains(&format!("muak.update_id={}", update_id)) {
        return UpdateStatus::Committed;
    }

    let marker_path = Path::new(UPDATE_DIR).join("pending-validation.json");
    if let Ok(contents) = fs::read_to_string(&marker_path)
        && let Ok(marker) = serde_json::from_str::<ValidationMarker>(&contents)
        && marker.update_id == update_id
    {
        return UpdateStatus::Pending;
    }

    UpdateStatus::Unknown
}

pub fn check_and_handle_pending_validation() -> Result<()> {
    let marker = match load_validation_marker()? {
        Some(m) => m,
        None => return Ok(()),
    };

    kmsg::info!(
        @ "provisioning",
        "Found pending validation for update {} -> {}",
        marker.current_image,
        marker.target_image
    );

    if is_old_kernel(&marker) {
        handle_kexec_failure(&marker)?;
    } else if let Err(e) = health_checks() {
        handle_validation_failure(&marker, e)?;
    } else {
        commit_update(&marker)?;
    }

    Ok(())
}

fn load_validation_marker() -> Result<Option<ValidationMarker>> {
    let marker_path = Path::new(UPDATE_DIR).join("pending-validation.json");

    if !marker_path.exists() {
        return Ok(None);
    }

    let contents =
        fs::read_to_string(&marker_path).context("Failed to read pending-validation.json")?;

    let marker: ValidationMarker =
        serde_json::from_str(&contents).context("Failed to parse pending-validation.json")?;

    Ok(Some(marker))
}

fn is_old_kernel(marker: &ValidationMarker) -> bool {
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let expected_marker = format!("muak.update_id={}", marker.update_id);

    !cmdline.contains(&expected_marker)
}

fn handle_kexec_failure(marker: &ValidationMarker) -> Result<()> {
    kmsg::info!(
        @ "provisioning",
        "Update {} failed - new kernel did not boot successfully",
        marker.update_id
    );

    rollback_update(marker, "Kernel failed to boot (kexec failure)")
}

fn handle_validation_failure(marker: &ValidationMarker, error: anyhow::Error) -> Result<()> {
    kmsg::info!(@ "provisioning", "Health checks failed: {}", error);
    rollback_update(marker, &format!("Health checks failed: {}", error))
}

fn health_checks() -> Result<()> {
    check_state_partition_writable()?;
    check_network_interfaces()?;
    Ok(())
}

fn check_state_partition_writable() -> Result<()> {
    let test_path = "/run/state/.update_health_check";
    fs::write(test_path, b"ok").context("STATE partition not writable")?;
    fs::remove_file(test_path).context("Failed to clean up health check file")?;
    Ok(())
}

fn check_network_interfaces() -> Result<()> {
    let net_dir = fs::read_dir("/sys/class/net").context("Failed to read network interfaces")?;

    let non_loopback_count = net_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "lo")
        .count();

    if non_loopback_count == 0 {
        anyhow::bail!("No non-loopback network interfaces found");
    }

    Ok(())
}

fn commit_update(marker: &ValidationMarker) -> Result<()> {
    kmsg::info!(
        @ "provisioning",
        "Validation succeeded, committing update {}",
        marker.update_id
    );

    let efi_device = disk::find_partition_by_partname("EFI")
        .ok_or_else(|| anyhow::anyhow!("EFI partition not found"))?;

    let mount_point = "/run/mnt/efi";
    mount_efi_partition(&efi_device, mount_point)?;

    let result = install_new_uki_and_finalize(marker, mount_point);

    unmount_partition(mount_point);

    result
}

fn install_new_uki_and_finalize(marker: &ValidationMarker, mount_point: &str) -> Result<()> {
    fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let uki = Uki::from_dir(Path::new(UPDATE_DIR));
    let uki_path = uki::get_uki_path(Path::new(mount_point))?;

    uki::build_atomic(&uki, &uki_path)?;

    if sysconfig::system().secureboot {
        let hierarchy = sbolt::keys::load_key_hierarchy(&Path::new(SECRETS_DIR).join("secureboot"))
            .context("Failed to load Secure Boot keys for UKI signing")?;
        uki::sign(&uki_path, &hierarchy)?;
    }

    update_config_image(&marker.target_image)?;

    cleanup_update_files();

    sync();

    Ok(())
}

fn update_config_image(new_image: &str) -> Result<()> {
    let contents = fs::read_to_string(CONFIG_PATH).context("Failed to read config.toml")?;
    let mut config: HostConfig =
        sysconfig::parse_from_str(&contents).context("Failed to parse config.toml")?;

    config.system.image = new_image.to_string();

    let updated_toml = sysconfig::serialize(&config).context("Failed to serialize config")?;
    fs::write(CONFIG_PATH, updated_toml).context("Failed to write updated config.toml")?;

    Ok(())
}

fn cleanup_update_files() {
    if let Err(e) = uki::cleanup_dir(Path::new(UPDATE_DIR)) {
        kmsg::warn!(
            @ "provisioning",
            "Failed to cleanup update work dir: {}",
            e
        );
    }
}

fn rollback_update(marker: &ValidationMarker, reason: &str) -> Result<()> {
    kmsg::info!(
        @ "provisioning",
        "Rolling back update {}: {}",
        marker.update_id,
        reason
    );

    save_rollback_info(marker, reason)?;
    cleanup_failed_update();

    sync();

    reboot(RebootCommand::Restart).context("Failed to reboot for rollback")?;

    unreachable!("If we're here, something went really wrong");
}

fn save_rollback_info(marker: &ValidationMarker, reason: &str) -> Result<()> {
    let rollbacks_dir = Path::new("/run/state/rollbacks");
    fs::create_dir_all(rollbacks_dir).context("Failed to create rollbacks dir")?;

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
    fs::write(path, data).context("Failed to write rollback info")?;

    Ok(())
}

fn cleanup_failed_update() {
    if let Err(e) = uki::cleanup_dir(Path::new(UPDATE_DIR)) {
        kmsg::warn!(
            @ "provisioning",
            "Failed to cleanup update work dir during rollback: {}",
            e
        );
    }
}
