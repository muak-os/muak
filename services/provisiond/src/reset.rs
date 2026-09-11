//! Factory reset functionality for removing STATE and DATA partitions.

use ::disk::role::Role;
use anyhow::{Result, bail};

use crate::disk;

/// Performs a factory reset by deleting the STATE and DATA partitions.
pub fn factory_reset() -> Result<()> {
    kmsg::info!("Starting factory reset...");

    let disk_config = &config::config().disk;
    let system_disk = disk_config.system.clone();
    if system_disk.is_empty() {
        bail!("System disk not configured");
    }

    disk::unmount_partition("/run/data")?;
    disk::unmount_partition("/run/state")?;

    if let Err(e) = luks2::close(Role::State.dm_name()) {
        kmsg::warn!("Failed to close LUKS STATE mapping (may not exist): {}", e);
    }
    if let Err(e) = luks2::close(Role::Data.dm_name()) {
        kmsg::warn!("Failed to close LUKS DATA mapping (may not exist): {}", e);
    }

    if disk_config.is_split() {
        delete_role(&system_disk, Role::State)?;
        delete_role(disk_config.data_disk(), Role::Data)?;
    } else {
        delete_role(&system_disk, Role::State)?;
        delete_role(&system_disk, Role::Data)?;
    }

    kmsg::info!("Factory reset complete");
    Ok(())
}

fn delete_role(disk: &str, role: Role) -> Result<()> {
    let name = role.gpt_name();
    match disk::find_partition_number(disk, name)? {
        Some(number) => disk::delete_partitions(disk, &[number])?,
        None => kmsg::info!("No '{name}' partition found on {disk}, nothing to delete"),
    }

    Ok(())
}
