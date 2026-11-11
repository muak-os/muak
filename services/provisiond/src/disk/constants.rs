//! Disk and partition size constants and type GUIDs.

pub(crate) use parttable::gpt::plan::SECTOR_SIZE;

pub(super) const MB: u64 = 1024 * 1024;
pub(super) const GB: u64 = 1024 * MB;

pub(super) const MIN_DISK_SIZE: u64 = 12 * GB;
