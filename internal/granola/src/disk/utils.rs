use anyhow::{Result, bail};
use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::path::Path;
use std::time::SystemTime;

use super::constants::MB;

pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
}

pub fn generate_guid() -> [u8; 16] {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    let mut guid = [0u8; 16];
    let nanos = now.as_nanos();
    guid[0..8].copy_from_slice(&nanos.to_le_bytes()[0..8]);
    guid[8..16].copy_from_slice(&nanos.to_be_bytes()[0..8]);

    guid
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

pub fn check_disk_not_mounted(disk: &str) -> Result<()> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();

    let disk_name = disk.trim_start_matches("/dev/");

    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let mount_device = parts[0];

        if mount_device.starts_with(disk) {
            let mount_point = parts.get(1).unwrap_or(&"unknown");
            bail!(
                "Cannot install: {} is mounted at {}. Unmount it first.",
                mount_device,
                mount_point
            );
        }

        if mount_device.contains(disk_name) {
            let mount_point = parts.get(1).unwrap_or(&"unknown");
            bail!(
                "Cannot install: {} is mounted at {}. Unmount it first.",
                mount_device,
                mount_point
            );
        }
    }

    Ok(())
}

pub fn wipe_disk(disk: &str) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    // Wipe first 10MB (removes primary GPT and any MBR/partition tables)
    f.seek(std::io::SeekFrom::Start(0))?;
    let zeros = vec![0u8; (10 * MB) as usize];
    f.write_all(&zeros)?;

    // Wipe last 1MB (removes backup GPT at end of disk)
    if disk_size > MB {
        let backup_start = disk_size - MB;
        f.seek(std::io::SeekFrom::Start(backup_start))?;
        let backup_zeros = vec![0u8; MB as usize];
        f.write_all(&backup_zeros)?;
    }

    f.sync_all()?;

    Ok(())
}
