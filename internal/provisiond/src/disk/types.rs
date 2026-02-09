//! Type definitions for disk and partition information.

/// Information about a partition on a disk.
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub number: u32,
    pub start_sector: u64,
    pub size_bytes: u64,
    pub name: String,
    pub path: String,
    pub fstype: String,
}

/// Information about a physical disk including all its partitions.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub model: String,
    pub removable: bool,
    pub read_only: bool,
    pub partitions: Vec<PartitionInfo>,
}

/// BLKPG ioctl argument structure for partition operations.
#[repr(C)]
pub(crate) struct BlkpgIoctlArg {
    pub op: i32,
    pub flags: i32,
    pub datalen: i32,
    pub data: *mut BlkpgPartition,
}

/// BLKPG partition data structure.
#[repr(C)]
pub(crate) struct BlkpgPartition {
    pub start: i64,
    pub length: i64,
    pub pno: i32,
    pub devname: [u8; 64],
    pub volname: [u8; 64],
}
