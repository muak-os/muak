use crate::log;
use anyhow::{Result, bail};
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::Path;

use super::constants::{GB, MB, MIN_DISK_SIZE, SECTOR_SIZE};
use super::types::{DiskInfo, PartitionInfo};

pub fn validate_disk_size(disk: &str) -> Result<()> {
    let mut f = File::open(disk)?;
    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    if disk_size < MIN_DISK_SIZE {
        bail!(
            "Disk '{}' is too small ({} MB). Minimum required: {} MB",
            disk,
            disk_size / MB,
            MIN_DISK_SIZE / MB
        );
    }

    log!(
        "installer",
        "Disk size: {} GB ({} MB)",
        disk_size / GB,
        disk_size / MB
    );

    Ok(())
}

pub fn validate_block_device(disk: &str) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = fs::metadata(disk)?;

    if !metadata.file_type().is_block_device() {
        bail!(
            "'{}' is not a block device. Please specify a disk (e.g., /dev/sda, /dev/vda)",
            disk
        );
    }

    if disk.chars().last().is_some_and(|c| c.is_numeric())
        && (disk.contains("sd") || disk.contains("vd") || disk.contains("hd"))
    {
        log!(
            "installer",
            "Warning: '{}' appears to be a partition. You should install to a whole disk (e.g., /dev/sda, not /dev/sda1)",
            disk
        );
    }

    Ok(())
}

pub fn find_partition_by_partname(partname: &str) -> Option<String> {
    let entries = fs::read_dir("/sys/class/block").ok()?;
    let target = format!("PARTNAME={}", partname);

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !entry.path().join("partition").exists() {
            continue;
        }

        let uevent = entry.path().join("uevent");
        let content = fs::read_to_string(&uevent).ok()?;
        let found = content.lines().any(|line| line.trim() == target);
        if !found {
            continue;
        }

        let dev_path = format!("/dev/{}", name);
        if Path::new(&dev_path).exists() {
            return Some(dev_path);
        }
    }

    None
}

fn is_physical_disk(name: &str) -> bool {
    !name.starts_with("loop")
        && !name.starts_with("dm-")
        && !name.starts_with("ram")
        && !name.starts_with("sr") // CD-ROM
}

fn read_sysfs_u64(path: &Path) -> Result<u64> {
    let content = fs::read_to_string(path)?;
    Ok(content.trim().parse()?)
}

fn read_sysfs_string(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)?;
    Ok(content.trim().to_string())
}

fn detect_filesystem(device_path: &str) -> String {
    let mut file = match File::open(device_path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };

    // Check for ext4 signature at offset 0x438 (superblock magic)
    // ext4 superblock starts at offset 1024 bytes
    // Magic bytes are at offset 0x38 within the superblock (so absolute offset 0x438)
    let mut ext4_magic = [0u8; 2];
    if file.seek(std::io::SeekFrom::Start(0x438)).is_ok()
        && file.read_exact(&mut ext4_magic).is_ok()
        && ext4_magic == [0x53, 0xEF]
    {
        return "ext4".to_string();
    }

    // Check for FAT signatures in boot sector
    if let Some(fstype) = detect_fat_filesystem(&mut file) {
        return fstype;
    }

    // Check for btrfs signature
    // btrfs superblock is at offset 0x10000 (65536 bytes)
    // Magic bytes "_BHRfS_M" are at offset 0x40 within the superblock (so absolute offset 0x10040)
    let mut btrfs_magic = [0u8; 8];
    if file.seek(std::io::SeekFrom::Start(0x10040)).is_ok()
        && file.read_exact(&mut btrfs_magic).is_ok()
        && btrfs_magic == *b"_BHRfS_M"
    {
        return "btrfs".to_string();
    }

    String::new()
}

fn detect_fat_filesystem(file: &mut File) -> Option<String> {
    if file.seek(std::io::SeekFrom::Start(0)).is_err() {
        return None;
    }

    let mut boot_sector = [0u8; 512];
    if file.read_exact(&mut boot_sector).is_err() {
        return None;
    }

    let fat32_sig = &boot_sector[82..90];
    if fat32_sig.starts_with(b"FAT32   ") {
        return Some("vfat".to_string());
    }

    let fat16_sig = &boot_sector[54..62];
    if fat16_sig.starts_with(b"FAT16   ") || fat16_sig.starts_with(b"FAT12   ") {
        return Some("vfat".to_string());
    }

    None
}

fn read_partition_info(disk_name: &str, part_name: &str) -> Result<PartitionInfo> {
    let sysfs_path = Path::new("/sys/block").join(disk_name).join(part_name);

    let number = read_sysfs_u64(&sysfs_path.join("partition"))? as u32;
    let start_sector = read_sysfs_u64(&sysfs_path.join("start"))?;
    let size_sectors = read_sysfs_u64(&sysfs_path.join("size"))?;
    let size_bytes = size_sectors * SECTOR_SIZE;

    let path = format!("/dev/{}", part_name);

    let fstype = detect_filesystem(&path);

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name: part_name.to_string(),
        path,
        fstype,
    })
}

fn read_disk_info(name: &str) -> Result<DiskInfo> {
    let sysfs_path = Path::new("/sys/block").join(name);

    let size_sectors = read_sysfs_u64(&sysfs_path.join("size"))?;
    let size_bytes = size_sectors * SECTOR_SIZE;
    let removable = read_sysfs_u64(&sysfs_path.join("removable"))? != 0;
    let read_only = read_sysfs_u64(&sysfs_path.join("ro"))? != 0;

    let model =
        read_sysfs_string(&sysfs_path.join("device/model")).unwrap_or_else(|_| name.to_string());

    let path = format!("/dev/{}", name);

    let partitions = discover_partitions(&sysfs_path, name);

    Ok(DiskInfo {
        name: name.to_string(),
        path,
        size_bytes,
        model,
        removable,
        read_only,
        partitions,
    })
}

fn discover_partitions(sysfs_path: &Path, disk_name: &str) -> Vec<PartitionInfo> {
    let Ok(entries) = fs::read_dir(sysfs_path) else {
        return Vec::new();
    };

    let mut partitions: Vec<_> = entries
        .flatten()
        .filter_map(|entry| try_read_partition(entry, disk_name))
        .collect();

    partitions.sort_by_key(|p| p.number);
    partitions
}

fn try_read_partition(entry: fs::DirEntry, disk_name: &str) -> Option<PartitionInfo> {
    let part_name = entry.file_name();
    let part_name_str = part_name.to_string_lossy();

    if !part_name_str.starts_with(disk_name) || part_name_str == disk_name {
        return None;
    }

    if !entry.path().join("partition").exists() {
        return None;
    }

    read_partition_info(disk_name, &part_name_str).ok()
}

pub fn list_disks() -> Result<Vec<DiskInfo>> {
    let block_dir = Path::new("/sys/block");
    if !block_dir.exists() {
        bail!("/sys/block does not exist - sysfs not mounted?");
    }

    let mut disks: Vec<_> = fs::read_dir(block_dir)?
        .flatten()
        .filter_map(try_read_disk)
        .collect();

    disks.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(disks)
}

fn try_read_disk(entry: fs::DirEntry) -> Option<DiskInfo> {
    let name = entry.file_name();
    let name_str = name.to_string_lossy();

    if !is_physical_disk(&name_str) {
        return None;
    }

    match read_disk_info(&name_str) {
        Ok(disk_info) if disk_info.size_bytes > 0 => Some(disk_info),
        Ok(_) => None,
        Err(e) => {
            log!("disk", "Failed to read disk {}: {}", name_str, e);
            None
        }
    }
}
