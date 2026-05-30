//! Btrfs filesystem creation and manipulation.

mod accessors;
mod builders;
mod checksum;
mod chunk;
mod context;
mod layout;
mod node;
mod structures;
mod trees;

use std::fs::File;
use std::os::fd::AsFd as _;

use context::MkfsContext;
use rustix::ioctl::{Getter, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::BLKGETSIZE64;

/// Format a device with Btrfs filesystem.
///
/// # Arguments
/// * `device` - File handle to the block device
/// * `label` - Filesystem label (max 256 bytes)
///
/// # Errors
/// Returns an error if the device size cannot be read or filesystem creation fails.
pub fn format(device: File, label: &str) -> Result<()> {
    let device_size = get_device_size(&device)?;

    let mut ctx = MkfsContext::new(device, device_size, label.to_owned());

    ctx.make_btrfs()
        .map_err(|error| BtrfsError::Mkfs(error.to_string()))?;

    Ok(())
}

/// Get device size in bytes using BLKGETSIZE64 ioctl.
///
/// # Errors
/// Returns an error if the block device ioctl fails.
pub fn get_device_size(device: &File) -> Result<u64> {
    // SAFETY: `BLKGETSIZE64` writes a `u64` and `Getter` carries the matching type.
    let getter = unsafe { Getter::<BLKGETSIZE64, u64>::new() };

    // SAFETY: The file descriptor is valid and the ioctl argument type matches the opcode.
    unsafe { ioctl(device.as_fd(), getter) }
        .map_err(|error| BtrfsError::Io(std::io::Error::from(error)))
}
