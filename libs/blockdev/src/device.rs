//! Block device path helpers and size probing.

use std::fs::{self, File};
use std::os::fd::AsFd as _;
use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;

use rustix::ioctl::{Getter, Opcode, ioctl, opcode};

use crate::error::Result;

// The `BLKGETSIZE64` ioctl: `_IOR(0x12, 114, size_t)`.
const BLKGETSIZE64: Opcode = opcode::read::<u64>(0x12, 114);

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

/// Returns the size in bytes of the block device or file behind `file`.
///
/// # Errors
///
/// Returns an error when the metadata lookup fails or a block device rejects the size ioctl.
pub fn size(file: &File) -> Result<u64> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_block_device() {
        return Ok(metadata.len());
    }

    // SAFETY: `BLKGETSIZE64` writes a `u64` and `Getter` carries the matching type.
    let getter = unsafe { Getter::<BLKGETSIZE64, u64>::new() };
    // SAFETY: the file descriptor is valid and the ioctl argument type matches the opcode.
    unsafe { ioctl(file.as_fd(), getter) }.map_err(|errno| std::io::Error::from(errno).into())
}
