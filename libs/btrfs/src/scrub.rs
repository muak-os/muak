//! Btrfs scrub operations.

use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use rustix::ioctl::{Updater, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::{
    BTRFS_DEVICE_PATH_NAME_MAX, BTRFS_IOC_DEV_INFO, BTRFS_IOC_FS_INFO, BTRFS_IOC_SCRUB,
    BTRFS_UUID_SIZE, BtrfsIoctlDevInfoArgs, BtrfsIoctlFsInfoArgs, BtrfsIoctlScrubArgs,
    BtrfsScrubProgress,
};

/// Start a scrub on a single device.
///
/// # Errors
/// Returns an error if the mount point cannot be opened or the scrub ioctl fails.
pub fn scrub(mount_point: &str, devid: u64, readonly: bool) -> Result<BtrfsScrubProgress> {
    let fd = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )?;

    let mut args = new_scrub_args(devid, readonly);

    // SAFETY: `BTRFS_IOC_SCRUB` updates `BtrfsIoctlScrubArgs` in place.
    let updater = unsafe { Updater::<BTRFS_IOC_SCRUB, BtrfsIoctlScrubArgs>::new(&mut args) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(&fd, updater) }.map_err(|source| BtrfsError::Scrub {
        mount_point: mount_point.to_owned(),
        source,
    })?;

    Ok(args.progress)
}

/// Get information about the filesystem, including device IDs.
///
/// # Errors
/// Returns an error if the mount point cannot be opened or filesystem ioctls fail.
pub fn get_fs_info(mount_point: &str) -> Result<Vec<u64>> {
    let fd = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )?;

    // Query filesystem info to learn device count and max device ID.
    let mut fs_info = BtrfsIoctlFsInfoArgs {
        max_id: 0,
        num_devices: 0,
        fsid: [0_u8; BTRFS_UUID_SIZE],
        nodesize: 0,
        sectorsize: 0,
        clone_alignment: 0,
        flags: 0,
        generation: 0,
        metadata_uuid: [0_u8; BTRFS_UUID_SIZE],
        reserved: [0_u8; 888],
    };

    // SAFETY: `BTRFS_IOC_FS_INFO` updates `BtrfsIoctlFsInfoArgs` in place.
    let updater = unsafe { Updater::<BTRFS_IOC_FS_INFO, BtrfsIoctlFsInfoArgs>::new(&mut fs_info) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(&fd, updater) }.map_err(|source| BtrfsError::Scrub {
        mount_point: mount_point.to_owned(),
        source,
    })?;

    let device_count = usize::try_from(fs_info.num_devices).map_err(|_error| {
        BtrfsError::InvalidArgument("device count does not fit usize".to_owned())
    })?;
    let mut device_ids = Vec::with_capacity(device_count);

    for candidate_id in 1..=fs_info.max_id {
        let mut dev_info = BtrfsIoctlDevInfoArgs {
            devid: candidate_id,
            uuid: [0_u8; BTRFS_UUID_SIZE],
            bytes_used: 0,
            total_bytes: 0,
            unused: [0_u64; 379],
            path: [0_u8; BTRFS_DEVICE_PATH_NAME_MAX],
        };

        // SAFETY: `BTRFS_IOC_DEV_INFO` updates `BtrfsIoctlDevInfoArgs` in place.
        let updater =
            unsafe { Updater::<BTRFS_IOC_DEV_INFO, BtrfsIoctlDevInfoArgs>::new(&mut dev_info) };

        // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
        let result = unsafe { ioctl(&fd, updater) };

        match result {
            Ok(()) => device_ids.push(candidate_id),
            Err(Errno::NODEV) => continue,
            Err(source) => {
                return Err(BtrfsError::Scrub {
                    mount_point: mount_point.to_owned(),
                    source,
                });
            }
        }

        if device_ids.len() == device_count {
            break;
        }
    }

    Ok(device_ids)
}

/// Build a zeroed `BtrfsIoctlScrubArgs` with the given device ID and flags.
#[must_use]
fn new_scrub_args(devid: u64, readonly: bool) -> BtrfsIoctlScrubArgs {
    BtrfsIoctlScrubArgs {
        devid,
        start: 0,
        end: u64::MAX,
        flags: u64::from(readonly),
        progress: BtrfsScrubProgress::default(),
        unused: [0_u64; 109],
    }
}
