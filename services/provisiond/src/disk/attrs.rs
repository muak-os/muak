//! Low-level sysfs attribute readers and filesystem probes.

use std::io::{Read as _, Seek as _};
use std::path::Path;

use anyhow::Result;
use tokio::fs;

use super::constants::SECTOR_SIZE;
use super::types::PartitionInfo;

/// Reads a trimmed u64 value from a sysfs attribute file.
pub(crate) async fn read_sysfs_u64(path: &Path) -> Result<u64> {
    let content = fs::read_to_string(path).await?;

    Ok(content.trim().parse()?)
}

/// Reads a trimmed string value from a sysfs attribute file.
pub(crate) async fn read_sysfs_string(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).await?;

    Ok(content.trim().to_owned())
}

/// Reads the file name of a symlink target, keeping the link unresolved.
pub(crate) fn read_link_name(path: &Path) -> Result<String> {
    let target = std::fs::read_link(path)?;
    let name = match target.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => String::new(),
    };

    Ok(name)
}

/// Reads the partition attributes of `part_name` on `disk_name` below `sysfs_root`.
pub(crate) async fn read_partition_info(
    sysfs_root: &Path,
    disk_name: &str,
    part_name: &str,
) -> Result<PartitionInfo> {
    let sysfs_path = sysfs_root.join(disk_name).join(part_name);

    let number = u32::try_from(read_sysfs_u64(&sysfs_path.join("partition")).await?).unwrap_or(0);
    let start_sector = read_sysfs_u64(&sysfs_path.join("start")).await?;
    let size_sectors = read_sysfs_u64(&sysfs_path.join("size")).await?;
    let size_bytes = size_sectors.saturating_mul(SECTOR_SIZE);

    let path = format!("/dev/{part_name}");

    let fstype = detect_filesystem(&path);

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name: part_name.to_owned(),
        path,
        fstype,
    })
}

fn detect_filesystem(device_path: &str) -> String {
    use std::fs::File;

    let Ok(mut file) = File::open(device_path) else {
        return String::new();
    };

    let mut ext4_magic = [0_u8; 2];
    if file.seek(std::io::SeekFrom::Start(0x438)).is_ok()
        && file.read_exact(&mut ext4_magic).is_ok()
        && ext4_magic == [0x53, 0xEF]
    {
        return "ext4".to_owned();
    }

    if let Some(fstype) = detect_fat_filesystem(&mut file) {
        return fstype;
    }

    let mut btrfs_magic = [0_u8; 8];
    if file.seek(std::io::SeekFrom::Start(0x10040)).is_ok()
        && file.read_exact(&mut btrfs_magic).is_ok()
        && btrfs_magic == *b"_BHRfS_M"
    {
        return "btrfs".to_owned();
    }

    String::new()
}

fn detect_fat_filesystem(file: &mut std::fs::File) -> Option<String> {
    if file.seek(std::io::SeekFrom::Start(0)).is_err() {
        return None;
    }

    let mut boot_sector = [0_u8; 512];
    if file.read_exact(&mut boot_sector).is_err() {
        return None;
    }

    let fat32_sig = &boot_sector[82..90];
    if fat32_sig.starts_with(b"FAT32   ") {
        return Some("vfat".to_owned());
    }

    let fat16_sig = &boot_sector[54..62];
    if fat16_sig.starts_with(b"FAT16   ") || fat16_sig.starts_with(b"FAT12   ") {
        return Some("vfat".to_owned());
    }

    None
}
