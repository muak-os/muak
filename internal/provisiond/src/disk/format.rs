//! Filesystem formatting utilities for EFI and Btrfs partitions.

use std::fs::OpenOptions;

use anyhow::{Context, Result};
use fatfs::{FatType, FormatVolumeOptions};

use super::utils::wait_for_device;

/// Formats a partition as FAT32 for EFI System Partition use.
pub fn format_efi_partition(device: &str) -> Result<()> {
    kmsg::info!("Formatting {} as FAT32", device);

    wait_for_device(device)?;

    let mut f = OpenOptions::new().read(true).write(true).open(device)?;

    fatfs::format_volume(
        &mut f,
        FormatVolumeOptions::new()
            .volume_label(*b"EFI        ") // 11 bytes, padded with spaces
            .fat_type(FatType::Fat32),
    )?;

    f.sync_all()?;

    kmsg::info!("FAT32 formatting complete");

    Ok(())
}

/// Formats a partition as Btrfs with the specified label.
pub fn format_btrfs_partition(device: &str, label: &str) -> Result<()> {
    kmsg::info!("Formatting {} as btrfs with label '{}'", device, label);

    wait_for_device(device)?;

    let f = OpenOptions::new().read(true).write(true).open(device)?;
    btrfs::format_btrfs(f, label).context("Failed to format partition as btrfs")?;

    kmsg::info!("btrfs formatting complete");

    Ok(())
}
