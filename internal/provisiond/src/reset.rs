//! Factory reset functionality for removing STATE and DATA partitions.

use anyhow::{Result, bail};

use crate::constants::{DM_DATA, DM_STATE};
use crate::disk;

/// Performs a factory reset by deleting STATE and DATA partitions.
pub fn factory_reset() -> Result<()> {
    kmsg::info!("Starting factory reset...");

    let disk = sysconfig::config().system.disk.clone();
    if disk.is_empty() {
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

    disk::delete_partitions(&disk, &[2, 3])?;

    kmsg::info!("Factory reset complete");
    Ok(())
}
