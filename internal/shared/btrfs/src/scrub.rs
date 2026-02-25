//! Btrfs scrub operations.

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Updater, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::{
    BTRFS_DEVICE_PATH_NAME_MAX, BTRFS_IOC_DEV_INFO, BTRFS_IOC_FS_INFO, BTRFS_IOC_SCRUB,
    BTRFS_UUID_SIZE, BtrfsIoctlDevInfoArgs, BtrfsIoctlFsInfoArgs, BtrfsIoctlScrubArgs,
    BtrfsScrubProgress,
};

/// Start a scrub on a single device.
pub fn scrub(mount_point: &str, devid: u64, readonly: bool) -> Result<BtrfsScrubProgress> {
    let fd = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )?;

    let mut args = new_scrub_args(devid, readonly);

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing.
    // BTRFS_IOC_SCRUB blocks until the scrub completes.
    unsafe {
        ioctl(
            &fd,
            Updater::<BTRFS_IOC_SCRUB, BtrfsIoctlScrubArgs>::new(&mut args),
        )
    }
    .map_err(|source| BtrfsError::Scrub {
        mount_point: mount_point.to_string(),
        source,
    })?;

    Ok(args.progress)
}

/// Get information about the filesystem, including device IDs.
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
        fsid: [0u8; BTRFS_UUID_SIZE],
        nodesize: 0,
        sectorsize: 0,
        clone_alignment: 0,
        flags: 0,
        generation: 0,
        metadata_uuid: [0u8; BTRFS_UUID_SIZE],
        reserved: [0u8; 888],
    };

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing.
    unsafe {
        ioctl(
            &fd,
            Updater::<BTRFS_IOC_FS_INFO, BtrfsIoctlFsInfoArgs>::new(&mut fs_info),
        )
    }
    .map_err(|source| BtrfsError::Scrub {
        mount_point: mount_point.to_string(),
        source,
    })?;

    // Iterate device IDs from 1..=max_id, collecting those that exist.
    // Not all IDs in the range may be valid (devices can be removed).
    let mut device_ids = Vec::with_capacity(fs_info.num_devices as usize);

    for candidate_id in 1..=fs_info.max_id {
        let mut dev_info = BtrfsIoctlDevInfoArgs {
            devid: candidate_id,
            uuid: [0u8; BTRFS_UUID_SIZE],
            bytes_used: 0,
            total_bytes: 0,
            unused: [0u64; 379],
            path: [0u8; BTRFS_DEVICE_PATH_NAME_MAX],
        };

        // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing.
        let result = unsafe {
            ioctl(
                &fd,
                Updater::<BTRFS_IOC_DEV_INFO, BtrfsIoctlDevInfoArgs>::new(&mut dev_info),
            )
        };

        match result {
            Ok(_) => device_ids.push(candidate_id),
            Err(rustix::io::Errno::NODEV) => continue,
            Err(source) => {
                return Err(BtrfsError::Scrub {
                    mount_point: mount_point.to_string(),
                    source,
                });
            }
        }

        if device_ids.len() == fs_info.num_devices as usize {
            break;
        }
    }

    Ok(device_ids)
}

/// Build a zeroed `BtrfsIoctlScrubArgs` with the given device ID and flags.
fn new_scrub_args(devid: u64, readonly: bool) -> BtrfsIoctlScrubArgs {
    BtrfsIoctlScrubArgs {
        devid,
        start: 0,
        end: u64::MAX,
        flags: if readonly { 1 } else { 0 },
        progress: BtrfsScrubProgress::default(),
        unused: [0u64; 109],
    }
}
