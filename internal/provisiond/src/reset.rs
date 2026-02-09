//! Factory reset functionality for removing STATE and DATA partitions.

use anyhow::{Result, bail};

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

    disk::delete_partitions(&disk, &[2, 3])?;

    kmsg::info!("Factory reset complete");
    Ok(())
}
