//! Block device path helpers.

use std::fs;
use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;

/// Returns true when `path` is a block device node.
#[must_use]
pub fn is_block(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_block_device())
}

/// Formats the kernel device path of partition `number` on `disk`.
#[must_use]
pub fn partition_path(disk: &str, number: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
}

/// Returns true when `path` looks like a partition of a SATA, virtio or IDE disk.
#[must_use]
pub fn looks_like_partition(path: &str) -> bool {
    path.chars().last().is_some_and(char::is_numeric)
        && (path.contains("sd") || path.contains("vd") || path.contains("hd"))
}
