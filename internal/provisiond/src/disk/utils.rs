//! Utility functions for disk operations including partitioning, mounting, and formatting.

use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use rustix::mount::{UnmountFlags, unmount};

use super::constants::MB;

/// Represents a mounted partition with device path and mount point.
pub struct MountedPartition {
    pub device: String,
    pub mount_point: String,
}

/// Formats a partition device path based on disk naming convention.
pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
}

/// Waits for a device node to appear in the filesystem.
pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0..30 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Timeout waiting for device {} to appear", device)
}

/// Retrieves all partitions mounted from the specified disk.
pub fn get_disk_mounts(disk: &str) -> Vec<MountedPartition> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();

    mounts
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let device = parts.next()?;
            let mount_point = parts.next()?;

            if !device.starts_with(disk) {
                return None;
            }

            Some(MountedPartition {
                device: device.to_string(),
                mount_point: mount_point.to_string(),
            })
        })
        .collect()
}

/// Unmounts all partitions in the provided list, deepest mount points first.
pub fn unmount_all(partitions: &[MountedPartition]) -> Result<()> {
    let mut sorted: Vec<_> = partitions.iter().collect();
    sorted.sort_by(|a, b| b.mount_point.len().cmp(&a.mount_point.len()));

    for p in sorted {
        unmount(p.mount_point.as_str(), UnmountFlags::empty())
            .with_context(|| format!("Failed to unmount {} from {}", p.device, p.mount_point))?;
    }

    Ok(())
}

/// Wipes the first and last portions of a disk to remove partition tables.
pub fn wipe_disk(disk: &str) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    // Wipe first 10MB (removes primary GPT and any MBR/partition tables)
    f.seek(std::io::SeekFrom::Start(0))?;
    let zeros = vec![0u8; (10 * MB) as usize];
    f.write_all(&zeros)?;

    // Wipe last 1MB (removes backup GPT at end of disk)
    if disk_size > MB {
        let backup_start = disk_size - MB;
        f.seek(std::io::SeekFrom::Start(backup_start))?;
        let backup_zeros = vec![0u8; MB as usize];
        f.write_all(&backup_zeros)?;
    }

    f.sync_all()?;

    Ok(())
}
