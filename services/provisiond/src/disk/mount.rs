//! Mount table reading and unmount helpers.

use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::sync;
use rustix::mount::{UnmountFlags, unmount};

use super::Targets;
use super::attrs::{MOUNTS_PATH, Mount, read_mounts};

/// Retrieves all partitions mounted from the specified disk.
pub fn get_disk_mounts(disk: &str) -> Vec<Mount> {
    read_mounts(Path::new(MOUNTS_PATH))
        .into_iter()
        .filter(|mount| mount.device.starts_with(disk))
        .collect()
}

/// Unmounts all partitions in the provided list, deepest mount points first.
pub fn unmount_all(partitions: &[Mount]) -> Result<()> {
    let mut sorted: Vec<_> = partitions.iter().collect();
    sorted.sort_by_key(|mount| core::cmp::Reverse(mount.mount_point.len()));

    for mount in sorted {
        unmount(mount.mount_point.as_str(), UnmountFlags::empty()).with_context(|| {
            format!(
                "Failed to unmount {} from {}",
                mount.device, mount.mount_point
            )
        })?;
    }

    Ok(())
}

/// Unmounts every filesystem on the targets so partitioning can proceed.
pub(crate) fn unmount_targets(targets: &Targets) -> Result<()> {
    for device in [targets.system.as_str(), targets.data()] {
        let mounted = get_disk_mounts(device);
        if mounted.is_empty() {
            continue;
        }

        sync();
        unmount_all(&mounted).with_context(|| format!("Failed to unmount '{device}'"))?;
    }

    Ok(())
}
