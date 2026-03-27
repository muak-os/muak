//! Utility functions for disk operations including partitioning, mounting, and formatting.

use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use rustix::fs::sync;
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

    f.seek(std::io::SeekFrom::Start(0))?;
    f.write_all(&vec![0u8; (10 * MB) as usize])?;

    if disk_size > MB {
        f.seek(std::io::SeekFrom::Start(disk_size - MB))?;
        f.write_all(&vec![0u8; MB as usize])?;
    }

    f.sync_all()?;

    Ok(())
}

/// Validates that the system and data disks are suitable install targets.
pub fn validate_install_target(system_disk: &str, data_disk: &str, force: bool) -> Result<()> {
    if !force && Path::new(config::CONFIG_PATH).exists() {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    validate_disk(system_disk, force)
        .with_context(|| format!("System disk '{}' failed validation", system_disk))?;

    if data_disk != system_disk {
        validate_disk(data_disk, force)
            .with_context(|| format!("Data disk '{}' failed validation", data_disk))?;
    }

    Ok(())
}

/// Validates a disk as a suitable install target.
fn validate_disk(disk_path: &str, force: bool) -> Result<()> {
    if !Path::new(disk_path).exists() {
        bail!("Disk '{}' does not exist", disk_path);
    }

    super::validate_block_device(disk_path)?;
    super::validate_disk_size(disk_path)?;

    let mounted = get_disk_mounts(disk_path);
    if !mounted.is_empty() && !force {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            mounted[0].device,
            mounted[0].mount_point
        );
    }

    sync();
    unmount_all(&mounted)?;

    let has_partitions = super::has_existing_partitions(disk_path)?;
    if has_partitions && !force {
        bail!(
            "Disk '{}' has existing partitions. Use --force to overwrite.",
            disk_path
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_partition_name_nvme_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/nvme0n1";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/nvme0n1p1");
    }

    #[test]
    fn format_partition_name_mmcblk_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/mmcblk0";

        // ACT
        let name = format_partition_name(disk, 2);

        // ASSERT
        assert_eq!(name, "/dev/mmcblk0p2");
    }

    #[test]
    fn format_partition_name_sda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/sda";

        // ACT
        let name = format_partition_name(disk, 3);

        // ASSERT
        assert_eq!(name, "/dev/sda3");
    }

    #[test]
    fn format_partition_name_vda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/vda";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/vda1");
    }
}
