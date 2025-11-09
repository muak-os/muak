use crate::log;
use anyhow::{Result, bail};
use fatfs::{FatType, FormatVolumeOptions, format_volume};
use std::fs::OpenOptions;
use std::process::Command;

use super::utils::wait_for_device;

pub fn format_efi_partition(device: &str) -> Result<()> {
    log!("installer", "Formatting {} as FAT32", device);

    wait_for_device(device)?;

    let mut f = OpenOptions::new().read(true).write(true).open(device)?;

    format_volume(
        &mut f,
        FormatVolumeOptions::new()
            .volume_label(*b"EFI        ") // 11 bytes, padded with spaces
            .fat_type(FatType::Fat32),
    )?;

    f.sync_all()?;

    log!("installer", "FAT32 formatting complete");

    Ok(())
}

pub fn format_ext4_partition(device: &str, label: &str) -> Result<()> {
    log!(
        "installer",
        "Formatting {} as ext4 with label '{}'",
        device,
        label
    );

    wait_for_device(device)?;

    let status = Command::new("mkfs.ext4")
        .arg("-F") // Force
        .arg("-L") // Label
        .arg(label)
        .arg(device)
        .status()?;

    if !status.success() {
        bail!("Failed to format {} as ext4", device);
    }

    log!("installer", "ext4 formatting complete");

    Ok(())
}
