//! Mount and unmount helpers for btrfs volumes.

#![warn(missing_docs)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rustix::fs::{Mode, mkdir, sync};
use rustix::io::Errno;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount as sys_unmount};

/// Mounts `device` as btrfs at `mount_point`, freeing the mount point first.
///
/// # Errors
///
/// Returns an error when `device` does not exist, when `mount_point` is mounted and
/// cannot be unmounted, or when the mount point or the mount itself fails.
pub fn ephemeral(device: &str, mount_point: &str) -> Result<()> {
    if !Path::new(device).exists() {
        bail!("EPHEMERAL device {device} does not exist");
    }

    sync();

    match sys_unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) | Err(Errno::NOENT | Errno::INVAL) => {}
        Err(error) => bail!(
            "Mount point {mount_point} is already mounted and could not be unmounted: {error}"
        ),
    }

    create_mount_point(mount_point)?;

    mount(device, mount_point, "btrfs", MountFlags::empty(), None)
        .with_context(|| format!("Failed to mount EPHEMERAL partition {device} at {mount_point}"))
}

/// Unmounts `mount_point`, logging instead of failing on errors.
pub fn unmount(mount_point: &str) {
    sync();

    match sys_unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) | Err(Errno::NOENT | Errno::INVAL) => {}
        Err(error) => kmsg::warn!("Failed to unmount {mount_point}: {error}"),
    }
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
}
