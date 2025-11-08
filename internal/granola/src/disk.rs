use crate::log;
use anyhow::{Result, bail};
use fatfs::{FatType, FormatVolumeOptions, format_volume};
use gptman::{GPT, GPTPartitionEntry};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

pub const SECTOR_SIZE: u64 = 512;
pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const EFI_SIZE: u64 = 512 * MB; // 512 MB for EFI
pub const STATE_SIZE: u64 = 1 * GB; // 1 GB for STATE
pub const MIN_DISK_SIZE: u64 = 2 * GB; // Minimum 2 GB total

// GPT Partition Type GUIDs
const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
]; // C12A7328-F81F-11D2-BA4B-00A0C93EC93B

const LINUX_FS_GUID: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
]; // 0FC63DAF-8483-4772-8E79-3D69D8477DE4

pub fn has_existing_partitions(disk: &str) -> Result<bool> {
    let mut f = File::open(disk)?;

    match GPT::find_from(&mut f) {
        Ok(gpt) => {
            let count = gpt.iter().count();
            Ok(count > 0)
        }
        Err(_) => Ok(false), // No valid GPT = no partitions
    }
}

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
    let metadata = fs::metadata(disk)?;

    if !metadata.file_type().is_block_device() {
        bail!(
            "'{}' is not a block device. Please specify a disk (e.g., /dev/sda, /dev/vda)",
            disk
        );
    }

    // Warn if it looks like a partition
    if disk.chars().last().unwrap().is_numeric()
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

pub fn wipe_disk(disk: &str) -> Result<()> {
    log!("installer", "Wiping disk {}", disk);

    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    // Wipe first 10MB (removes any existing partition tables)
    let zeros = vec![0u8; (10 * MB) as usize];
    f.write_all(&zeros)?;
    f.sync_all()?;

    Ok(())
}

pub fn create_partitions(disk: &str) -> Result<(String, String, String)> {
    log!("installer", "Creating GPT partition table on {}", disk);

    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    // Get disk size
    let disk_size = f.seek(std::io::SeekFrom::End(0))?;
    f.seek(std::io::SeekFrom::Start(0))?;

    log!("installer", "Disk size: {} GB", disk_size / GB);

    // Create new GPT
    let mut gpt = GPT::new_from(&mut f, SECTOR_SIZE, [0xff; 16])?;

    // Calculate partition sizes in sectors
    let efi_sectors = EFI_SIZE / SECTOR_SIZE;
    let state_sectors = STATE_SIZE / SECTOR_SIZE;

    // Get usable LBA range
    let first_usable = gpt.header.first_usable_lba;
    let last_usable = gpt.header.last_usable_lba;

    // Partition 1: EFI
    let efi_start = first_usable;
    let efi_end = efi_start + efi_sectors - 1;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: generate_guid(),
        starting_lba: efi_start,
        ending_lba: efi_end,
        attribute_bits: 0,
        partition_name: "EFI".try_into().unwrap(),
    };

    // Partition 2: STATE
    let state_start = efi_end + 1;
    let state_end = state_start + state_sectors - 1;

    gpt[2] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: generate_guid(),
        starting_lba: state_start,
        ending_lba: state_end,
        attribute_bits: 0,
        partition_name: "STATE".try_into().unwrap(),
    };

    // Partition 3: DATA (rest of disk)
    let data_start = state_end + 1;
    let data_end = last_usable;

    gpt[3] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: generate_guid(),
        starting_lba: data_start,
        ending_lba: data_end,
        attribute_bits: 0,
        partition_name: "DATA".try_into().unwrap(),
    };

    // Write GPT to disk
    gpt.write_into(&mut f)?;
    f.sync_all()?;

    log!("installer", "GPT partition table created successfully");

    // Construct partition device names
    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);
    let data_part = format_partition_name(disk, 3);

    // Wait for kernel to re-read partition table
    std::thread::sleep(std::time::Duration::from_secs(2));

    Ok((efi_part, state_part, data_part))
}

pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
}

fn generate_guid() -> [u8; 16] {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    let mut guid = [0u8; 16];
    let nanos = now.as_nanos();
    guid[0..8].copy_from_slice(&nanos.to_le_bytes()[0..8]);
    guid[8..16].copy_from_slice(&nanos.to_be_bytes()[0..8]);

    guid
}

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

pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0..30 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Timeout waiting for device {} to appear", device)
}

#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub number: u32,
    pub start_sector: u64,
    pub size_bytes: u64,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub model: String,
    pub removable: bool,
    pub read_only: bool,
    pub partitions: Vec<PartitionInfo>,
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

fn read_partition_info(disk_name: &str, part_name: &str) -> Result<PartitionInfo> {
    let sysfs_path = Path::new("/sys/block").join(disk_name).join(part_name);

    let number = read_sysfs_u64(&sysfs_path.join("partition"))? as u32;
    let start_sector = read_sysfs_u64(&sysfs_path.join("start"))?;
    let size_sectors = read_sysfs_u64(&sysfs_path.join("size"))?;
    let size_bytes = size_sectors * SECTOR_SIZE;

    let path = format!("/dev/{}", part_name);

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name: part_name.to_string(),
        path,
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

    // Find partitions by listing subdirectories that start with the disk name
    let mut partitions = Vec::new();
    if let Ok(entries) = fs::read_dir(&sysfs_path) {
        for entry in entries.flatten() {
            let part_name = entry.file_name();
            let part_name_str = part_name.to_string_lossy();

            // Check if this is a partition
            if part_name_str.starts_with(name) && part_name_str != name {
                let partition_file = entry.path().join("partition");
                if partition_file.exists() {
                    if let Ok(part_info) = read_partition_info(name, &part_name_str) {
                        partitions.push(part_info);
                    }
                }
            }
        }
    }

    partitions.sort_by_key(|p| p.number);

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

pub fn list_disks() -> Result<Vec<DiskInfo>> {
    let mut disks = Vec::new();

    let block_dir = Path::new("/sys/block");
    if !block_dir.exists() {
        bail!("/sys/block does not exist - sysfs not mounted?");
    }

    let entries = fs::read_dir(block_dir)?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if is_physical_disk(&name_str) {
            match read_disk_info(&name_str) {
                Ok(disk_info) => {
                    if disk_info.size_bytes > 0 {
                        disks.push(disk_info);
                    }
                }
                Err(e) => {
                    log!("disk", "Failed to read disk {}: {}", name_str, e);
                }
            }
        }
    }

    disks.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(disks)
}
