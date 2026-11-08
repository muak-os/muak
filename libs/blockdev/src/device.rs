//! Block device path helpers and size probing.

use std::fs;
use std::os::fd::AsFd;
use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;

use crate::error::Result;
use crate::kernel::Kernel as _;

/// Returns true when `path` is a block device node.
#[must_use]
pub fn is_block(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_block_device())
}

/// Returns the size in bytes of the block device or file behind `fd`.
///
/// # Errors
///
/// Returns an error when the descriptor lookup fails, the descriptor reports a
/// negative size, or a block device rejects the size ioctl.
pub fn size<Fd: AsFd>(fd: &Fd) -> Result<u64> {
    crate::handle().size(fd)
}

/// Formats the kernel device path of partition `number` on `disk`.
#[must_use]
pub fn partition_path(disk: &str, number: u32) -> String {
    if uses_p_separator(disk) {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
}

/// Returns true when `device` is `disk` itself or a partition of that disk.
#[must_use]
pub fn belongs_to(device: &str, disk: &str) -> bool {
    if device == disk {
        return true;
    }

    let Some(suffix) = device.strip_prefix(disk) else {
        return false;
    };

    if uses_p_separator(disk) {
        suffix.strip_prefix('p').is_some_and(is_partition_number)
    } else {
        is_partition_number(suffix)
    }
}

fn uses_p_separator(disk: &str) -> bool {
    disk.chars()
        .last()
        .is_some_and(|character| character.is_ascii_digit())
}

fn is_partition_number(suffix: &str) -> bool {
    !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
}
