//! Type definitions for kernel partition table operations.

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
