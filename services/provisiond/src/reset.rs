//! Factory reset functionality for removing STATE and DATA partitions.

use ::disk::role::Role;
use anyhow::{Context as _, Result};

use crate::disk;

/// Performs a factory reset by deleting the STATE and DATA partitions.
pub fn factory_reset() -> Result<()> {
    kmsg::info!("Starting factory reset...");

    disk::unmount_partition("/run/data")?;
    disk::unmount_partition("/run/state")?;

    if let Err(e) = luks2::close(Role::State.dm_name()) {
        kmsg::warn!("Failed to close LUKS STATE mapping (may not exist): {}", e);
    }
    if let Err(e) = luks2::close(Role::Data.dm_name()) {
        kmsg::warn!("Failed to close LUKS DATA mapping (may not exist): {}", e);
    }

    let state_device = disk::find_partition_device(Role::State)
        .context("STATE partition not found on any disk")?;
    let state_disk = disk::parent_disk(&state_device)
        .context("Failed to resolve the disk carrying the STATE partition")?;
    delete_role(&state_disk, Role::State)?;

    match disk::find_partition_device(Role::Data) {
        Some(data_device) => {
            let data_disk = disk::parent_disk(&data_device)
                .context("Failed to resolve the disk carrying the DATA partition")?;
            if data_disk == state_disk {
                delete_role(&state_disk, Role::Data)?;
            } else {
                delete_role(&data_disk, Role::Data)?;
            }
        }
        None => kmsg::info!("No DATA partition found, nothing to delete"),
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
