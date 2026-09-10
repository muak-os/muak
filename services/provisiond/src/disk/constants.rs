//! Disk and partition size constants and type GUIDs.

pub(crate) use parttable::gpt::plan::SECTOR_SIZE;

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const MIN_DISK_SIZE: u64 = 2 * GB;
