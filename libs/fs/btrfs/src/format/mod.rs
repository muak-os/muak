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

use context::MkfsContext;

use crate::error::{BtrfsError, Result};

/// Format a device with Btrfs filesystem.
///
/// # Arguments
/// * `device` - File handle to the block device
/// * `label` - Filesystem label (max 256 bytes)
///
/// # Errors
/// Returns an error if the device size cannot be read or filesystem creation fails.
pub fn format(device: File, label: &str) -> Result<()> {
    let device_size = blockdev::device::size(&device)?;

    let mut ctx = MkfsContext::new(device, device_size, label.to_owned());

    ctx.make_btrfs()
        .map_err(|error| BtrfsError::Mkfs(error.to_string()))?;
    ctx.sync_device()?;

    Ok(())
}
