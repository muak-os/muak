//! Partition lookup by GPT name and resolution of the disk owning a partition.

use crate::handle;
use crate::kernel::Kernel as _;

/// Finds the partition device paths carrying the GPT partition name `partname`.
#[must_use]
pub fn find_by_partname(partname: &str) -> Vec<String> {
    handle().find_by_partname(partname)
}

/// Returns the name of the disk carrying `partition_device`, for example `nvme0n1`.
#[must_use]
pub fn parent_disk(partition_device: &str) -> Option<String> {
    handle().parent_disk(partition_device)
}
