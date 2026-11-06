//! Filesystem formatting utilities for EFI and Btrfs partitions.

use core::time::Duration;
use std::fs::OpenOptions;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

// Wait for a device node to appear.
pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0_i32..30_i32 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    bail!("Timeout waiting for device {device} to appear")
}

/// Formats a partition as FAT32 for EFI System Partition use.
///
/// # Errors
///
/// Returns an error when `size_bytes` is zero, when the device does not appear,
/// or when the format itself fails.
pub fn efi(device: &str, size_bytes: u64) -> Result<()> {
    if size_bytes == 0 {
        bail!("Refusing to format {device} as EFI FAT32: partition size is zero");
    }

    kmsg::info!("Formatting {} as FAT32", device);

    wait_for_device(device)?;

    let mut file = OpenOptions::new().read(true).write(true).open(device)?;

    fatfs::builder::format(&mut file, size_bytes)
        .context("Failed to format partition as EFI FAT32")?;

    file.sync_all()?;

    kmsg::info!("FAT32 formatting complete");

    Ok(())
}

/// Formats a partition as Btrfs with the specified label.
pub fn btrfs(device: &str, label: &str) -> Result<()> {
    kmsg::info!("Formatting {} as btrfs with label '{}'", device, label);

    wait_for_device(device)?;

    let file = OpenOptions::new().read(true).write(true).open(device)?;
    btrfs::format::format(file, label).context("Failed to format partition as btrfs")?;

    kmsg::info!("btrfs formatting complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::{Read as _, Write as _};

    use tempfile::NamedTempFile;

    use super::*;

    // Sector size reported by the FAT boot sector.
    const SECTOR_BYTES: u64 = 512;
    // Chosen ESP size in sectors, deliberately different from the frozen default.
    const PLACED_SECTORS: u64 = 0x0002_0000;
    // Chosen ESP size in bytes.
    const PLACED_SIZE: u64 = PLACED_SECTORS * SECTOR_BYTES;

    fn sized_file(size: u64) -> NamedTempFile {
        let file = NamedTempFile::new().expect("temp file");
        file.as_file().set_len(size).expect("resize");
        file
    }

    fn boot_sector(path: &Path) -> [u8; 512] {
        let mut boot = [0_u8; 512];
        File::open(path)
            .expect("open image")
            .read_exact(&mut boot)
            .expect("read boot sector");

        boot
    }

    #[test]
    fn efi_sizes_the_volume_from_the_placed_partition() {
        // ARRANGE
        let image = sized_file(PLACED_SIZE);

        // ACT
        efi(
            image.path().to_str().expect("temp path is utf-8"),
            PLACED_SIZE,
        )
        .expect("format must succeed");

        // ASSERT
        let boot = boot_sector(image.path());
        assert_eq!(
            boot.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "a formatted volume carries the FAT boot signature"
        );
        assert_eq!(
            boot.get(82..90),
            Some(&b"FAT32   "[..]),
            "the placed size lands in the FAT32 range"
        );
        let total_sectors = boot
            .get(32..36)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_le_bytes)
            .expect("total sectors field");
        assert_eq!(
            u64::from(total_sectors),
            PLACED_SECTORS,
            "the BPB must describe the passed size, not the frozen default"
        );
    }

    #[test]
    fn efi_rejects_a_zero_size_without_touching_the_device() {
        // ARRANGE
        let image = sized_file(4096);
        let marker = [0x5A_u8; 4096];
        image.as_file().write_all(&marker).expect("seed the device");

        // ACT
        let result = efi(image.path().to_str().expect("temp path is utf-8"), 0);

        // ASSERT
        let error = result.expect_err("a zero-sized volume must be rejected");
        assert!(
            error.to_string().contains("zero"),
            "the rejection must name the zero size: {error}"
        );
        let mut untouched = [0_u8; 4096];
        OpenOptions::new()
            .read(true)
            .open(image.path())
            .expect("reopen")
            .read_exact(&mut untouched)
            .expect("read back");
        assert_eq!(untouched, marker, "a rejected format must not write");
    }
}
