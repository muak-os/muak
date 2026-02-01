//! Quota management for Btrfs subvolumes.

use std::mem::size_of;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Setter, Updater, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::{
    BTRFS_FIRST_FREE_OBJECTID, BTRFS_IOC_INO_LOOKUP, BTRFS_IOC_QGROUP_LIMIT, BTRFS_IOC_QUOTA_CTL,
    BTRFS_IOC_TREE_SEARCH, BTRFS_QGROUP_INFO_KEY, BTRFS_QGROUP_LIMIT_KEY,
    BTRFS_QGROUP_LIMIT_MAX_RFER, BTRFS_QUOTA_CTL_ENABLE, BTRFS_QUOTA_TREE_OBJECTID,
    BTRFS_SEARCH_ARGS_BUFSIZE, InoLookupArgs, QgroupInfoItem, QgroupLimit, QgroupLimitArgs,
    QgroupLimitItem, QuotaCtlArgs, SearchArgs, SearchHeader, SearchKey,
};

/// Disk usage information for a subvolume.
#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub usage_percent: f32,
}

/// Enable Btrfs quota on a mounted partition.
pub fn enable_quota(mount_point: &str) -> Result<()> {
    let file = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )?;

    let mut args = QuotaCtlArgs {
        cmd: BTRFS_QUOTA_CTL_ENABLE,
        status: 0,
    };

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing
    unsafe {
        ioctl(
            &file,
            Updater::<BTRFS_IOC_QUOTA_CTL, QuotaCtlArgs>::new(&mut args),
        )
    }
    .map_err(|source| BtrfsError::QuotaEnable {
        mount_point: mount_point.to_string(),
        source,
    })?;

    Ok(())
}

/// Set a quota limit on a subvolume.
pub fn set_quota(vm_id: &str, size_bytes: u64, data_dir: &str) -> Result<()> {
    let path = PathBuf::from(data_dir).join(vm_id);

    let file = open(&path, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;

    let args = QgroupLimitArgs {
        qgroupid: 0,
        lim: QgroupLimit {
            flags: BTRFS_QGROUP_LIMIT_MAX_RFER,
            max_rfer: size_bytes,
            max_excl: 0,
            rsv_rfer: 0,
            rsv_excl: 0,
        },
    };

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing.
    unsafe {
        ioctl(
            &file,
            Setter::<BTRFS_IOC_QGROUP_LIMIT, QgroupLimitArgs>::new(args),
        )
    }
    .map_err(|source| BtrfsError::QuotaLimit {
        path: path.clone(),
        source,
    })?;

    Ok(())
}

/// Get the disk usage for a subvolume.
///
/// # Errors
/// Returns an error if any of the ioctls fail.
pub fn get_usage(vm_id: &str, data_dir: &str) -> Result<DiskUsage> {
    let path = PathBuf::from(data_dir).join(vm_id);

    let file = open(&path, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;

    let subvol_id = get_subvolume_id(&file, &path)?;

    // Search the quota tree for both info (242) and limit (244) items in one call.
    let mut args = SearchArgs {
        key: SearchKey {
            tree_id: BTRFS_QUOTA_TREE_OBJECTID,
            min_objectid: subvol_id,
            max_objectid: subvol_id,
            min_offset: 0,
            max_offset: u64::MAX,
            min_transid: 0,
            max_transid: u64::MAX,
            min_type: BTRFS_QGROUP_INFO_KEY,
            max_type: BTRFS_QGROUP_LIMIT_KEY,
            nr_items: 2,
            unused: 0,
            unused1: 0,
            unused2: 0,
            unused3: 0,
            unused4: 0,
        },
        buf: [0u8; BTRFS_SEARCH_ARGS_BUFSIZE],
    };

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing
    unsafe {
        ioctl(
            &file,
            Updater::<BTRFS_IOC_TREE_SEARCH, SearchArgs>::new(&mut args),
        )
    }
    .map_err(|source| BtrfsError::QuotaLookup {
        path: path.clone(),
        source,
    })?;

    let mut used_bytes = 0u64;
    let mut quota_bytes = 0u64;
    let mut offset = 0usize;

    for _ in 0..args.key.nr_items {
        if offset + size_of::<SearchHeader>() > BTRFS_SEARCH_ARGS_BUFSIZE {
            break;
        }

        // SAFETY: We verified bounds above and SearchHeader is repr(C).
        let header = unsafe { &*(args.buf.as_ptr().add(offset).cast::<SearchHeader>()) };
        offset += size_of::<SearchHeader>();

        let data_end = offset + header.len as usize;
        if data_end > BTRFS_SEARCH_ARGS_BUFSIZE {
            break;
        }

        match header.type_ {
            BTRFS_QGROUP_INFO_KEY if header.len as usize >= size_of::<QgroupInfoItem>() => {
                // SAFETY: Bounds checked, QgroupInfoItem is repr(C).
                let info = unsafe { &*(args.buf.as_ptr().add(offset).cast::<QgroupInfoItem>()) };
                used_bytes = info.rfer;
            }
            BTRFS_QGROUP_LIMIT_KEY if header.len as usize >= size_of::<QgroupLimitItem>() => {
                // SAFETY: Bounds checked, QgroupLimitItem is repr(C).
                let limit = unsafe { &*(args.buf.as_ptr().add(offset).cast::<QgroupLimitItem>()) };
                quota_bytes = limit.max_rfer;
            }
            _ => {}
        }

        offset = data_end;
    }

    let usage_percent = calculate_usage_percent(used_bytes, quota_bytes);

    Ok(DiskUsage {
        used_bytes,
        quota_bytes,
        usage_percent,
    })
}

/// Obtain the subvolume ID for an open directory fd via `BTRFS_IOC_INO_LOOKUP`.
fn get_subvolume_id(file: &rustix::fd::OwnedFd, path: &Path) -> Result<u64> {
    let mut args = InoLookupArgs {
        treeid: 0,
        objectid: BTRFS_FIRST_FREE_OBJECTID,
        name: [0u8; 4080],
    };

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing
    unsafe {
        ioctl(
            file,
            Updater::<BTRFS_IOC_INO_LOOKUP, InoLookupArgs>::new(&mut args),
        )
    }
    .map_err(|source| BtrfsError::QuotaLookup {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(args.treeid)
}

fn calculate_usage_percent(used: u64, quota: u64) -> f32 {
    if quota > 0 {
        (used as f64 / quota as f64 * 100.0) as f32
    } else {
        0.0
    }
}
