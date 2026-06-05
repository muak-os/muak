//! Quota management for Btrfs subvolumes.

use core::mem::size_of;
use std::path::{Path, PathBuf};

use rustix::fd::OwnedFd;
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

const SEARCH_HEADER_TYPE_OFFSET: usize = 24;
const SEARCH_HEADER_LEN_OFFSET: usize = 28;
const QGROUP_RFER_OFFSET: usize = 8;
const QGROUP_MAX_RFER_OFFSET: usize = 8;

/// Disk usage information for a subvolume.
#[derive(Debug, Clone, Default)]
pub struct DiskUsage {
    /// Bytes currently used.
    pub used_bytes: u64,
    /// Quota limit in bytes.
    pub quota_bytes: u64,
    /// Usage as a percentage.
    pub usage_percent: u16,
}

/// Enable Btrfs quota on a mounted partition.
///
/// # Errors
/// Returns an error if the mount point cannot be opened or the quota ioctl fails.
pub fn enable(mount_point: &str) -> Result<()> {
    let file = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )?;

    let mut args = QuotaCtlArgs {
        cmd: BTRFS_QUOTA_CTL_ENABLE,
        status: 0,
    };

    // SAFETY: `BTRFS_IOC_QUOTA_CTL` updates `QuotaCtlArgs` in place.
    let updater = unsafe { Updater::<BTRFS_IOC_QUOTA_CTL, QuotaCtlArgs>::new(&mut args) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(&file, updater) }.map_err(|source| BtrfsError::QuotaEnable {
        mount_point: mount_point.to_owned(),
        source,
    })?;

    Ok(())
}

/// Set a quota limit on a subvolume.
///
/// # Errors
/// Returns an error if the subvolume cannot be opened or the quota ioctl fails.
pub fn set(vm_id: &str, size_bytes: u64, data_dir: &str) -> Result<()> {
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

    // SAFETY: `BTRFS_IOC_QGROUP_LIMIT` expects `QgroupLimitArgs` by value.
    let setter = unsafe { Setter::<BTRFS_IOC_QGROUP_LIMIT, QgroupLimitArgs>::new(args) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(&file, setter) }.map_err(|source| BtrfsError::QuotaLimit {
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
        buf: [0_u8; BTRFS_SEARCH_ARGS_BUFSIZE],
    };

    // SAFETY: `BTRFS_IOC_TREE_SEARCH` updates `SearchArgs` in place.
    let updater = unsafe { Updater::<BTRFS_IOC_TREE_SEARCH, SearchArgs>::new(&mut args) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(&file, updater) }.map_err(|source| BtrfsError::QuotaLookup {
        path: path.clone(),
        source,
    })?;

    let mut used_bytes = 0_u64;
    let mut quota_bytes = 0_u64;
    let mut offset = 0_usize;

    for _ in 0..args.key.nr_items {
        let Some(header_end) = offset.checked_add(size_of::<SearchHeader>()) else {
            break;
        };

        let Some(header) = parse_search_header(&args.buf, offset) else {
            break;
        };
        offset = header_end;

        let Some(data_end) = offset.checked_add(header.len) else {
            break;
        };
        if data_end > BTRFS_SEARCH_ARGS_BUFSIZE {
            break;
        }

        let Some(data) = args.buf.get(offset..data_end) else {
            break;
        };

        match header.type_ {
            BTRFS_QGROUP_INFO_KEY if header.len >= size_of::<QgroupInfoItem>() => {
                used_bytes = read_u64_le(data, QGROUP_RFER_OFFSET).unwrap_or(used_bytes);
            }
            BTRFS_QGROUP_LIMIT_KEY if header.len >= size_of::<QgroupLimitItem>() => {
                quota_bytes = read_u64_le(data, QGROUP_MAX_RFER_OFFSET).unwrap_or(quota_bytes);
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
fn get_subvolume_id(file: &OwnedFd, path: &Path) -> Result<u64> {
    let mut args = InoLookupArgs {
        treeid: 0,
        objectid: BTRFS_FIRST_FREE_OBJECTID,
        name: [0_u8; 4080],
    };

    // SAFETY: `BTRFS_IOC_INO_LOOKUP` updates `InoLookupArgs` in place.
    let updater = unsafe { Updater::<BTRFS_IOC_INO_LOOKUP, InoLookupArgs>::new(&mut args) };

    // SAFETY: The directory file descriptor is valid and the ioctl argument type matches.
    unsafe { ioctl(file, updater) }.map_err(|source| BtrfsError::QuotaLookup {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(args.treeid)
}

fn calculate_usage_percent(used: u64, quota: u64) -> u16 {
    let Some(percent) = used.saturating_mul(100).checked_div(quota) else {
        return 0;
    };

    u16::try_from(percent).unwrap_or(u16::MAX)
}

struct ParsedSearchHeader {
    type_: u32,
    len: usize,
}

fn parse_search_header(buf: &[u8], offset: usize) -> Option<ParsedSearchHeader> {
    let header = buf.get(offset..offset.checked_add(size_of::<SearchHeader>())?)?;
    let type_ = read_u32_le(header, SEARCH_HEADER_TYPE_OFFSET)?;
    let len = usize::try_from(read_u32_le(header, SEARCH_HEADER_LEN_OFFSET)?).ok()?;

    Some(ParsedSearchHeader { type_, len })
}

fn read_u32_le(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes = buf.get(offset..offset.checked_add(size_of::<u32>())?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u64_le(buf: &[u8], offset: usize) -> Option<u64> {
    let bytes = buf.get(offset..offset.checked_add(size_of::<u64>())?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}
