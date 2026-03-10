//! Factory reset functionality for removing STATE and DATA partitions.

use anyhow::{Result, bail};

use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;

/// Performs a factory reset by deleting STATE and DATA partitions.
pub fn factory_reset() -> Result<()> {
    kmsg::info!("Starting factory reset...");

    let disk_config = &config::config().disk;
    let system_disk = disk_config.system.clone();
    if system_disk.is_empty() {
        bail!("System disk not configured");
    }

    disk::unmount_partition("/run/data")?;
    disk::unmount_partition("/run/state")?;

    if let Err(e) = luks2::close(DM_DATA) {
        kmsg::warn!("Failed to close LUKS DATA mapping (may not exist): {}", e);
    }
    if let Err(e) = luks2::close(DM_STATE) {
        kmsg::warn!("Failed to close LUKS STATE mapping (may not exist): {}", e);
    }

    if disk_config.is_split() {
        let data_disk = disk_config.data_disk().to_string();
        disk::delete_partitions(&system_disk, &[2])?;
        disk::delete_partitions(&data_disk, &[1])?;
    } else {
        disk::delete_partitions(&system_disk, &[2, 3])?;
    }

    kmsg::info!("Factory reset complete");
    Ok(())
}
