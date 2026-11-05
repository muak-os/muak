//! Mount table reading and mount/unmount helpers.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rustix::fs::{Mode, mkdir, sync};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

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

/// Mounts the EFI System Partition at `mount_point`.
pub(crate) fn efi(device: &str, mount_point: &str) -> Result<()> {
    kmsg::info!("Mounting EFI partition {} at {}", device, mount_point);

    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {mount_point}"))?;

    mount(device, mount_point, "vfat", MountFlags::NOATIME, None)
        .with_context(|| format!("Failed to mount EFI partition {device} at {mount_point}"))
}

/// Mounts `device` as btrfs at `mount_point`, freeing the mount point first.
pub(crate) fn ephemeral(device: &str, mount_point: &str) -> Result<()> {
    if !Path::new(device).exists() {
        bail!("EPHEMERAL device {device} does not exist");
    }

    sync();

    match unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) | Err(Errno::NOENT | Errno::INVAL) => {}
        Err(error) => bail!(
            "Mount point {mount_point} is already mounted and could not be unmounted: {error}"
        ),
    }

    create_mount_point(mount_point)?;

    mount(device, mount_point, "btrfs", MountFlags::empty(), None)
        .with_context(|| format!("Failed to mount EPHEMERAL partition {device} at {mount_point}"))
}

/// Unmounts `mount_point`, logging a warning instead of returning an error.
pub(crate) fn try_unmount(mount_point: &str) {
    if let Err(error) = unmount_partition(mount_point) {
        kmsg::warn!("Failed to unmount {}: {}", mount_point, error);
    }
}

/// Unmounts `mount_point`, treating an absent mount as success.
pub(crate) fn unmount_partition(mount_point: &str) -> Result<()> {
    kmsg::info!("Unmounting {}", mount_point);
    sync();

    match unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) => {
            kmsg::info!("Unmounted {}", mount_point);
            Ok(())
        }
        Err(Errno::NOENT | Errno::INVAL) => {
            kmsg::warn!("{} not mounted, skipping", mount_point);
            Ok(())
        }
        Err(error) => bail!("Failed to unmount {mount_point}: {error}"),
    }
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

fn create_mount_point(path: &str) -> Result<()> {
    match mkdir(path, Mode::RWXU | Mode::RWXG | Mode::ROTH) {
        Ok(()) | Err(Errno::EXIST) => Ok(()),
        Err(error) => bail!("Failed to create mount point {path}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn mount_point_creation_is_idempotent() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = tmp.path().join("ephemeral");
        let mount_point = path.to_str().expect("utf8 path");

        // ACT
        create_mount_point(mount_point).expect("first creation");
        create_mount_point(mount_point).expect("second creation");

        // ASSERT
        assert!(path.is_dir(), "the mount point must exist");
    }

    #[test]
    fn ephemeral_rejects_missing_device() {
        // ARRANGE
        let device = "/dev/mapper/nonexistent-ephemeral";
        let mount_point = "/run/nonexistent-ephemeral";

        // ACT
        let result = ephemeral(device, mount_point);

        // ASSERT
        let error = result.expect_err("missing device must be rejected");
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn unmounting_an_absent_mount_succeeds() {
        // ARRANGE
        let mount_point = "/run/nonexistent-mount";

        // ACT
        let result = unmount_partition(mount_point);

        // ASSERT
        assert!(result.is_ok(), "an absent mount must unmount successfully");
    }
}
