use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rustix::fs::sync;

/// Validates that the system and data disks are suitable install targets.
pub fn install_target(system_disk: &str, data_disk: &str, force: bool) -> Result<()> {
    if !force && Path::new(config::CONFIG_PATH).exists() {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    disk(system_disk, force)
        .with_context(|| format!("System disk '{system_disk}' failed validation"))?;

    if data_disk != system_disk {
        disk(data_disk, force)
            .with_context(|| format!("Data disk '{data_disk}' failed validation"))?;
    }

    Ok(())
}

/// Validates a disk as a suitable install target.
fn disk(disk_path: &str, force: bool) -> Result<()> {
    if !Path::new(disk_path).exists() {
        bail!("Disk '{disk_path}' does not exist");
    }

    super::validate_block_device(disk_path)?;
    super::validate_disk_size(disk_path)?;

    let mounted = super::mount::get_disk_mounts(disk_path);
    if !mounted.is_empty()
        && !force
        && let Some(first) = mounted.first()
    {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            first.device,
            first.mount_point
        );
    }

    sync();
    super::mount::unmount_all(&mounted)?;

    let has_state_partition = super::has_state_partition(disk_path)?;
    if has_state_partition && !force {
        bail!(
            "Disk '{disk_path}' already has a Muak installation (STATE partition found). \
             Use --force to overwrite."
        );
    }

    if super::manage::disk_is_non_empty(disk_path)? && !force {
        bail!("Disk '{disk_path}' is not empty and will be overwritten. Use --force to continue.");
    }

    Ok(())
}
