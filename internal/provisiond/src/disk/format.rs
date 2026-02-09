//! Filesystem formatting utilities for EFI and Btrfs partitions.

use std::fs::OpenOptions;
use std::process::Command;

use anyhow::{Result, bail};
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

    let output = Command::new("/sbin/mkfs.btrfs")
        .arg("-f")
        .arg("-L")
        .arg(label)
        .arg(device)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to format {} as btrfs: {}", device, stderr);
    }

    kmsg::info!("btrfs formatting complete");

    Ok(())
}
