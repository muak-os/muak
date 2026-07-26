//! Filesystem formatting utilities for EFI and Btrfs partitions.

use std::fs::OpenOptions;
use std::path::Path;

use anyhow::{Context, Result, bail};

// Wait for a device node to appear.
pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0..30 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    bail!("Timeout waiting for device {} to appear", device)
}

/// Formats a partition as FAT32 for EFI System Partition use.
pub fn format_efi_partition(device: &str) -> Result<()> {
    kmsg::info!("Formatting {} as FAT32", device);

    wait_for_device(device)?;

    let mut file = OpenOptions::new().read(true).write(true).open(device)?;

    fatfs::builder::format(&mut file, crate::disk::constants::EFI_SIZE)
        .context("Failed to format partition as EFI FAT32")?;

    file.sync_all()?;

    kmsg::info!("FAT32 formatting complete");

    Ok(())
}

/// Formats a partition as Btrfs with the specified label.
pub fn format_btrfs_partition(device: &str, label: &str) -> Result<()> {
    kmsg::info!("Formatting {} as btrfs with label '{}'", device, label);

    wait_for_device(device)?;

    let file = OpenOptions::new().read(true).write(true).open(device)?;
    btrfs::format::format(file, label).context("Failed to format partition as btrfs")?;

    kmsg::info!("btrfs formatting complete");

    Ok(())
}
