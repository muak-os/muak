//! Btrfs filesystem creation and manipulation.

mod accessors;
mod builders;
mod checksum;
mod chunk;
mod constants;
mod context;
mod layout;
mod node;
mod structures;

mod trees;

use std::fs::File;
use std::os::fd::AsFd;

pub use context::MkfsContext;
use rustix::ioctl::{Getter, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::BLKGETSIZE64;

/// Format a device with Btrfs filesystem.
///
/// # Arguments
/// * `device` - File handle to the block device
/// * `label` - Filesystem label (max 256 bytes)
///
/// # Returns
/// `Ok(())` on success, error otherwise
pub fn format_btrfs(device: File, label: &str) -> Result<()> {
    let device_size = get_device_size(&device)?;

    let mut ctx = MkfsContext::new(device, device_size, label.to_string());

    ctx.make_btrfs()
        .map_err(|e| BtrfsError::Mkfs(e.to_string()))?;

    Ok(())
}

/// Get device size in bytes using BLKGETSIZE64 ioctl.
pub fn get_device_size(device: &File) -> Result<u64> {
    // SAFETY: ioctl is inherently unsafe, but Getter ensures proper argument passing.
    unsafe { ioctl(device.as_fd(), Getter::<BLKGETSIZE64, u64>::new()) }
        .map_err(|e| BtrfsError::Io(std::io::Error::from(e)))
}
