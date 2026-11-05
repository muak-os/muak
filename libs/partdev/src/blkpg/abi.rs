//! ABI definitions for the BLKPG partition ioctl match the kernel's `blkpg_*`.

// BLKPG ioctl argument structure for partition operations.
#[repr(C)]
pub(super) struct BlkpgIoctlArg {
    pub(super) op: i32,
    pub(super) flags: i32,
    pub(super) datalen: i32,
    pub(super) data: *mut BlkpgPartition,
}

// BLKPG partition data structure.
#[repr(C)]
pub(super) struct BlkpgPartition {
    pub(super) start: i64,
    pub(super) length: i64,
    pub(super) pno: i32,
    pub(super) devname: [u8; 64],
    pub(super) volname: [u8; 64],
}
