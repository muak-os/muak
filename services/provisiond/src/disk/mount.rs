//! Partition mounting and unmounting utilities.

use anyhow::{Context as _, Result, bail};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

/// Represents a mounted partition with device path and mount point.
pub struct MountedPartition {
    pub device: String,
    pub mount_point: String,
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
                device: device.to_owned(),
                mount_point: mount_point.to_owned(),
            })
        })
        .collect()
}

/// Mounts the EFI System Partition at the specified mount point.
pub fn mount_efi_partition(efi_device: &str, mount_point: &str) -> Result<()> {
    kmsg::info!("Mounting EFI partition {} at {}", efi_device, mount_point);

    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {mount_point}"))?;

    mount(efi_device, mount_point, "vfat", MountFlags::NOATIME, None)
        .with_context(|| format!("Failed to mount EFI partition {efi_device} at {mount_point}"))?;

    Ok(())
}

/// Unmounts all partitions in the provided list, deepest mount points first.
pub fn unmount_all(partitions: &[MountedPartition]) -> Result<()> {
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

/// Unmounts a partition, logging a warning on failure instead of returning an error.
pub fn try_unmount(mount_point: &str) {
    if let Err(e) = unmount(mount_point, UnmountFlags::empty()) {
        kmsg::warn!("Failed to unmount {}: {}", mount_point, e);
    }
}

/// Unmounts a partition at the specified mount point.
pub fn unmount_partition(mount_point: &str) -> Result<()> {
    kmsg::info!("Unmounting {}", mount_point);

    match unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) => {
            kmsg::info!("Unmounted {}", mount_point);
            Ok(())
        }
        Err(Errno::NOENT | Errno::INVAL) => {
            kmsg::warn!("{} not mounted, skipping", mount_point);
            Ok(())
        }
        Err(e) => {
            bail!("Failed to unmount {mount_point}: {e}");
        }
    }
}
