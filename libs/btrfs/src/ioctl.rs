//! Btrfs ioctl definitions and utilities.

use core::mem::size_of;

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
    /// File descriptor of the subvolume.
    pub fd: i64,
    /// Name of the subvolume.
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
    /// Quota control command.
    pub cmd: u64,
    /// Quota control status.
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
    /// Limit flags.
    pub flags: u64,
    /// Maximum referenced size in bytes.
    pub max_rfer: u64,
    /// Maximum exclusive size in bytes.
    pub max_excl: u64,
    /// Reserved referenced size.
    pub rsv_rfer: u64,
    /// Reserved exclusive size.
    pub rsv_excl: u64,
}

/// Arguments for the qgroup limit ioctl.
#[repr(C)]
pub struct QgroupLimitArgs {
    /// Qgroup ID.
    pub qgroupid: u64,
    /// Qgroup limit parameters.
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
    /// Tree ID of the subvolume.
    pub treeid: u64,
    /// Object ID for lookup.
    pub objectid: u64,
    /// Path name buffer.
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
    /// Tree ID to search.
    pub tree_id: u64,
    /// Minimum object ID.
    pub min_objectid: u64,
    /// Maximum object ID.
    pub max_objectid: u64,
    /// Minimum offset.
    pub min_offset: u64,
    /// Maximum offset.
    pub max_offset: u64,
    /// Minimum transaction ID.
    pub min_transid: u64,
    /// Maximum transaction ID.
    pub max_transid: u64,
    /// Minimum key type.
    pub min_type: u32,
    /// Maximum key type.
    pub max_type: u32,
    /// Number of items to return.
    pub nr_items: u32,
    /// Padding (unused).
    pub unused: u32,
    /// Padding (unused).
    pub unused1: u64,
    /// Padding (unused).
    pub unused2: u64,
    /// Padding (unused).
    pub unused3: u64,
    /// Padding (unused).
    pub unused4: u64,
}

/// Buffer size for tree search results (v1).
pub const BTRFS_SEARCH_ARGS_BUFSIZE: usize = 4096 - size_of::<SearchKey>();

/// Arguments for tree search ioctl (v1).
#[repr(C)]
pub struct SearchArgs {
    /// Search key parameters.
    pub key: SearchKey,
    /// Result buffer.
    pub buf: [u8; BTRFS_SEARCH_ARGS_BUFSIZE],
}

/// Header for each item in search results.
#[repr(C)]
pub struct SearchHeader {
    /// Transaction ID.
    pub transid: u64,
    /// Object ID.
    pub objectid: u64,
    /// Offset.
    pub offset: u64,
    /// Key type.
    pub type_: u32,
    /// Data length.
    pub len: u32,
}

/// Qgroup info item (on-disk format in quota tree, key type 242).
#[repr(C)]
pub struct QgroupInfoItem {
    /// Generation number.
    pub generation: u64,
    /// Referenced space in bytes.
    pub rfer: u64,
    /// Compressed referenced space in bytes.
    pub rfer_cmpr: u64,
    /// Exclusive space in bytes.
    pub excl: u64,
    /// Compressed exclusive space in bytes.
    pub excl_cmpr: u64,
}

/// Qgroup limit item (on-disk format in quota tree, key type 244).
#[repr(C)]
pub struct QgroupLimitItem {
    /// Limit flags.
    pub flags: u64,
    /// Maximum referenced size in bytes.
    pub max_rfer: u64,
    /// Maximum exclusive size in bytes.
    pub max_excl: u64,
    /// Reserved referenced size.
    pub rsv_rfer: u64,
    /// Reserved exclusive size.
    pub rsv_excl: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrub ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for starting a scrub operation.
pub const BTRFS_IOC_SCRUB: Opcode =
    opcode::read_write::<BtrfsIoctlScrubArgs>(BTRFS_IOCTL_MAGIC, 27);

/// Scrub progress statistics reported by the kernel.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct BtrfsScrubProgress {
    /// Data extents scrubbed.
    pub data_extents_scrubbed: u64,
    /// Tree extents scrubbed.
    pub tree_extents_scrubbed: u64,
    /// Data bytes scrubbed.
    pub data_bytes_scrubbed: u64,
    /// Tree bytes scrubbed.
    pub tree_bytes_scrubbed: u64,
    /// Read errors encountered.
    pub read_errors: u64,
    /// Checksum errors encountered.
    pub csum_errors: u64,
    /// Verify errors encountered.
    pub verify_errors: u64,
    /// Extents with no checksum.
    pub no_csum: u64,
    /// Checksum discards.
    pub csum_discards: u64,
    /// Super block errors.
    pub super_errors: u64,
    /// Memory allocation errors.
    pub malloc_errors: u64,
    /// Uncorrectable errors.
    pub uncorrectable_errors: u64,
    /// Corrected errors.
    pub corrected_errors: u64,
    /// Last physical address scrubbed.
    pub last_physical: u64,
    /// Unverified errors.
    pub unverified_errors: u64,
}

impl BtrfsScrubProgress {
    /// Returns `true` if any errors were detected during the scrub.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.read_errors > 0
            || self.csum_errors > 0
            || self.verify_errors > 0
            || self.super_errors > 0
            || self.uncorrectable_errors > 0
            || self.unverified_errors > 0
    }

    /// Total number of errors of all types.
    #[must_use]
    pub fn total_errors(&self) -> u64 {
        [
            self.read_errors,
            self.csum_errors,
            self.verify_errors,
            self.super_errors,
            self.uncorrectable_errors,
            self.unverified_errors,
        ]
        .into_iter()
        .fold(0_u64, u64::saturating_add)
    }
}

/// Padding element count (109).
const SCRUB_ARGS_UNUSED: usize = 109;

/// Arguments for the scrub ioctls.
#[repr(C)]
pub struct BtrfsIoctlScrubArgs {
    /// Device ID.
    pub devid: u64,
    /// Start byte offset.
    pub start: u64,
    /// End byte offset.
    pub end: u64,
    /// Scrub flags.
    pub flags: u64,
    /// Scrub progress statistics.
    pub progress: BtrfsScrubProgress,
    /// Padding (unused).
    pub unused: [u64; SCRUB_ARGS_UNUSED],
}

// ─────────────────────────────────────────────────────────────────────────────
// Filesystem info ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// Ioctl for querying filesystem info.
pub const BTRFS_IOC_FS_INFO: Opcode = opcode::read::<BtrfsIoctlFsInfoArgs>(BTRFS_IOCTL_MAGIC, 31);

/// Ioctl for querying device info.
pub const BTRFS_IOC_DEV_INFO: Opcode =
    opcode::read_write::<BtrfsIoctlDevInfoArgs>(BTRFS_IOCTL_MAGIC, 30);

/// Maximum UUID size in bytes.
pub const BTRFS_UUID_SIZE: usize = 16;

/// Maximum device path length.
pub const BTRFS_DEVICE_PATH_NAME_MAX: usize = 1024;

/// Filesystem info returned by `BTRFS_IOC_FS_INFO`.
#[repr(C)]
pub struct BtrfsIoctlFsInfoArgs {
    /// Maximum device ID.
    pub max_id: u64,
    /// Number of devices.
    pub num_devices: u64,
    /// Filesystem UUID.
    pub fsid: [u8; BTRFS_UUID_SIZE],
    /// Node size.
    pub nodesize: u32,
    /// Sector size.
    pub sectorsize: u32,
    /// Clone alignment.
    pub clone_alignment: u32,
    /// Filesystem flags.
    pub flags: u16,
    /// Filesystem generation.
    pub generation: u64,
    /// Metadata UUID.
    pub metadata_uuid: [u8; BTRFS_UUID_SIZE],
    /// Reserved (unused).
    pub reserved: [u8; 888],
}

/// Device info returned by `BTRFS_IOC_DEV_INFO`.
#[repr(C)]
pub struct BtrfsIoctlDevInfoArgs {
    /// Device ID.
    pub devid: u64,
    /// Device UUID.
    pub uuid: [u8; BTRFS_UUID_SIZE],
    /// Bytes used on device.
    pub bytes_used: u64,
    /// Total device size in bytes.
    pub total_bytes: u64,
    /// Padding (unused).
    pub unused: [u64; 379],
    /// Device path.
    pub path: [u8; BTRFS_DEVICE_PATH_NAME_MAX],
}

// ─────────────────────────────────────────────────────────────────────────────
// Block device ioctls
// ─────────────────────────────────────────────────────────────────────────────

/// BLKGETSIZE64 opcode: `_IOR(0x12, 114, size_t)`.
pub const BLKGETSIZE64: Opcode = opcode::read::<u64>(0x12, 114);
