//! Btrfs ioctl definitions and utilities.

use std::mem::size_of;

use rustix::ioctl::{Opcode, opcode};

/// Btrfs ioctl magic number.
pub const BTRFS_IOCTL_MAGIC: u8 = 0x94;

/// Maximum path name length for subvolume operations.
pub const BTRFS_PATH_NAME_MAX: usize = 4087;

// ─────────────────────────────────────────────────────────────────────────────
// Subvolume ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for creating a subvolume.
pub const BTRFS_IOC_SUBVOL_CREATE: Opcode = opcode::write::<VolArgs>(BTRFS_IOCTL_MAGIC, 14);

/// Ioctl for destroying (deleting) a subvolume/snapshot.
pub const BTRFS_IOC_SNAP_DESTROY: Opcode = opcode::write::<VolArgs>(BTRFS_IOCTL_MAGIC, 15);

/// Arguments for subvolume/snapshot create/destroy ioctls.
#[repr(C)]
pub struct VolArgs {
    pub fd: i64,
    pub name: [u8; BTRFS_PATH_NAME_MAX + 1],
}

// ─────────────────────────────────────────────────────────────────────────────
// Quota control ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for quota control operations.
pub const BTRFS_IOC_QUOTA_CTL: Opcode = opcode::read_write::<QuotaCtlArgs>(BTRFS_IOCTL_MAGIC, 40);

/// Enable quota command.
pub const BTRFS_QUOTA_CTL_ENABLE: u64 = 1;

/// Arguments for quota control ioctl.
#[repr(C)]
pub struct QuotaCtlArgs {
    pub cmd: u64,
    pub status: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Qgroup limit ioctl
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for setting qgroup limits.
///
/// Kernel defines this as `_IOR(0x94, 43)` but it actually writes data
/// from userspace to kernel -- a known btrfs encoding inconsistency.
pub const BTRFS_IOC_QGROUP_LIMIT: Opcode = opcode::read::<QgroupLimitArgs>(BTRFS_IOCTL_MAGIC, 43);

/// Flag: apply `max_rfer` limit.
pub const BTRFS_QGROUP_LIMIT_MAX_RFER: u64 = 1 << 0;

/// Qgroup limit parameters.
#[repr(C)]
pub struct QgroupLimit {
    pub flags: u64,
    pub max_rfer: u64,
    pub max_excl: u64,
    pub rsv_rfer: u64,
    pub rsv_excl: u64,
}

/// Arguments for the qgroup limit ioctl.
#[repr(C)]
pub struct QgroupLimitArgs {
    pub qgroupid: u64,
    pub lim: QgroupLimit,
}

// ─────────────────────────────────────────────────────────────────────────────
// Inode lookup ioctl (used to obtain a subvolume ID)
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for inode lookup.
pub const BTRFS_IOC_INO_LOOKUP: Opcode = opcode::read_write::<InoLookupArgs>(BTRFS_IOCTL_MAGIC, 18);

/// First free object ID -- used as `objectid` input to get the subvolume ID.
pub const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;

const BTRFS_INO_LOOKUP_PATH_MAX: usize = 4080;

/// Arguments for the inode lookup ioctl.
#[repr(C)]
pub struct InoLookupArgs {
    pub treeid: u64,
    pub objectid: u64,
    pub name: [u8; BTRFS_INO_LOOKUP_PATH_MAX],
}

// ─────────────────────────────────────────────────────────────────────────────
// Tree search ioctl (v1) -- used for reading quota tree entries
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for tree search (v1).
pub const BTRFS_IOC_TREE_SEARCH: Opcode = opcode::read_write::<SearchArgs>(BTRFS_IOCTL_MAGIC, 17);

/// Quota tree object ID.
pub const BTRFS_QUOTA_TREE_OBJECTID: u64 = 8;

/// Qgroup info item key type.
pub const BTRFS_QGROUP_INFO_KEY: u32 = 242;

/// Qgroup limit item key type.
pub const BTRFS_QGROUP_LIMIT_KEY: u32 = 244;

/// Search key for tree search ioctl.
#[repr(C)]
pub struct SearchKey {
    pub tree_id: u64,
    pub min_objectid: u64,
    pub max_objectid: u64,
    pub min_offset: u64,
    pub max_offset: u64,
    pub min_transid: u64,
    pub max_transid: u64,
    pub min_type: u32,
    pub max_type: u32,
    pub nr_items: u32,
    pub unused: u32,
    pub unused1: u64,
    pub unused2: u64,
    pub unused3: u64,
    pub unused4: u64,
}

/// Buffer size for tree search results (v1).
pub const BTRFS_SEARCH_ARGS_BUFSIZE: usize = 4096 - size_of::<SearchKey>();

/// Arguments for tree search ioctl (v1).
#[repr(C)]
pub struct SearchArgs {
    pub key: SearchKey,
    pub buf: [u8; BTRFS_SEARCH_ARGS_BUFSIZE],
}

/// Header for each item in search results.
#[repr(C)]
pub struct SearchHeader {
    pub transid: u64,
    pub objectid: u64,
    pub offset: u64,
    pub type_: u32,
    pub len: u32,
}

/// Qgroup info item (on-disk format in quota tree, key type 242).
#[repr(C)]
pub struct QgroupInfoItem {
    pub generation: u64,
    pub rfer: u64,
    pub rfer_cmpr: u64,
    pub excl: u64,
    pub excl_cmpr: u64,
}

/// Qgroup limit item (on-disk format in quota tree, key type 244).
#[repr(C)]
pub struct QgroupLimitItem {
    pub flags: u64,
    pub max_rfer: u64,
    pub max_excl: u64,
    pub rsv_rfer: u64,
    pub rsv_excl: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Block device ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// BLKGETSIZE64 opcode: `_IOR(0x12, 114, size_t)`
pub const BLKGETSIZE64: Opcode = opcode::read::<u64>(0x12, 114);
