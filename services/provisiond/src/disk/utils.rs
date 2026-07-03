//! Utility functions for disk operations including partitioning, mounting, and formatting.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use parttable::gpt::table::Table;
use parttable::mbr::types::{
    MBR_BOOT_SIGNATURE, MBR_BYTES, MBR_ENTRY_SIZE, MBR_PARTITION_ENTRY_OFFSET,
};
use rustix::fs::sync;
use rustix::mount::{UnmountFlags, unmount};

use super::constants::MB;

const MBR_MAX_SLOTS: usize = 4;
const MBR_PARTITION_TYPE_OFFSET: usize = 4;

/// Represents a mounted partition with device path and mount point.
pub struct MountedPartition {
    pub device: String,
    pub mount_point: String,
}

/// Formats a partition device path based on disk naming convention.
pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
}

/// Waits for a device node to appear in the filesystem.
pub fn wait_for_device(device: &str) -> Result<()> {
    for _ in 0..30 {
        if Path::new(device).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("Timeout waiting for device {} to appear", device)
}

/// Retrieves all partitions mounted from the specified disk.
pub fn get_disk_mounts(disk: &str) -> Vec<MountedPartition> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();

    mounts
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let device = parts.next()?;
            let mount_point = parts.next()?;

            if !device.starts_with(disk) {
                return None;
            }

            Some(MountedPartition {
                device: device.to_string(),
                mount_point: mount_point.to_string(),
            })
        })
        .collect()
}

/// Unmounts all partitions in the provided list, deepest mount points first.
pub fn unmount_all(partitions: &[MountedPartition]) -> Result<()> {
    let mut sorted: Vec<_> = partitions.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.mount_point.len()));

    for p in sorted {
        unmount(p.mount_point.as_str(), UnmountFlags::empty())
            .with_context(|| format!("Failed to unmount {} from {}", p.device, p.mount_point))?;
    }

    Ok(())
}

/// Wipes the first and last portions of a disk to remove partition tables.
pub fn wipe_disk(disk: &str) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    f.seek(std::io::SeekFrom::Start(0))?;
    f.write_all(&vec![0u8; (10 * MB) as usize])?;

    if disk_size > MB {
        f.seek(std::io::SeekFrom::Start(disk_size - MB))?;
        f.write_all(&vec![0u8; MB as usize])?;
    }

    f.sync_all()?;

    Ok(())
}

/// Returns `true` when `disk` contains GPT or MBR partition state.
pub fn disk_is_non_empty(disk: &str) -> Result<bool> {
    let mut f = OpenOptions::new().read(true).open(disk)?;

    if Table::read(&mut f).is_ok() {
        return Ok(true);
    }

    f.seek(SeekFrom::Start(0))?;

    let mut sector = [0u8; MBR_BYTES];
    if f.read_exact(&mut sector).is_err() {
        return Ok(false);
    }

    let boot_sig = [sector[510], sector[511]];
    if boot_sig != MBR_BOOT_SIGNATURE {
        return Ok(false);
    }

    Ok((0..MBR_MAX_SLOTS).any(|slot| {
        let entry_offset = MBR_PARTITION_ENTRY_OFFSET as usize + slot * MBR_ENTRY_SIZE;
        sector[entry_offset + MBR_PARTITION_TYPE_OFFSET] != 0x00
    }))
}

/// Validates that the system and data disks are suitable install targets.
pub fn validate_install_target(system_disk: &str, data_disk: &str, force: bool) -> Result<()> {
    if !force && Path::new(config::CONFIG_PATH).exists() {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    validate_disk(system_disk, force)
        .with_context(|| format!("System disk '{}' failed validation", system_disk))?;

    if data_disk != system_disk {
        validate_disk(data_disk, force)
            .with_context(|| format!("Data disk '{}' failed validation", data_disk))?;
    }

    Ok(())
}

/// Validates a disk as a suitable install target.
fn validate_disk(disk_path: &str, force: bool) -> Result<()> {
    if !Path::new(disk_path).exists() {
        bail!("Disk '{}' does not exist", disk_path);
    }

    super::validate_block_device(disk_path)?;
    super::validate_disk_size(disk_path)?;

    let mounted = get_disk_mounts(disk_path);
    if !mounted.is_empty() && !force {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            mounted[0].device,
            mounted[0].mount_point
        );
    }

    sync();
    unmount_all(&mounted)?;

    let has_state_partition = super::has_state_partition(disk_path)?;
    if has_state_partition && !force {
        bail!(
            "Disk '{}' already has a Muak installation (STATE partition found). \
             Use --force to overwrite.",
            disk_path
        );
    }

    if disk_is_non_empty(disk_path)? && !force {
        bail!(
            "Disk '{}' is not empty and will be overwritten. Use --force to continue.",
            disk_path
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use parttable::mbr;
    use parttable::mbr::types::MbrPartitionEntry;
    use tempfile::NamedTempFile;

    use super::*;

    fn disk_with_contents(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("temp file");
        f.write_all(bytes).expect("write");
        f
    }

    #[test]
    fn format_partition_name_nvme_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/nvme0n1";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/nvme0n1p1");
    }

    #[test]
    fn format_partition_name_mmcblk_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/mmcblk0";

        // ACT
        let name = format_partition_name(disk, 2);

        // ASSERT
        assert_eq!(name, "/dev/mmcblk0p2");
    }

    #[test]
    fn format_partition_name_sda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/sda";

        // ACT
        let name = format_partition_name(disk, 3);

        // ASSERT
        assert_eq!(name, "/dev/sda3");
    }

    #[test]
    fn format_partition_name_vda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/vda";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/vda1");
    }

    #[test]
    fn disk_is_non_empty_returns_false_for_zeroed_disk() {
        // ARRANGE
        let disk = disk_with_contents(&[0; 4096]);

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn disk_is_non_empty_returns_true_for_gpt_disk() {
        // ARRANGE
        let disk = NamedTempFile::new().expect("temp file");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        file.set_len(64 * 1024 * 1024).expect("resize");
        let gpt = Table::create(
            u64::try_from(file.metadata().expect("metadata").len()).unwrap_or(0) / 512,
            512,
            [0xff; 16],
        )
        .expect("new gpt");
        gpt.write_primary_to(
            u64::try_from(file.metadata().expect("metadata").len()).unwrap_or(0) / 512,
            &mut file,
        )
        .expect("write primary gpt");
        gpt.write_backup_to(
            u64::try_from(file.metadata().expect("metadata").len()).unwrap_or(0) / 512,
            &mut file,
        )
        .expect("write backup gpt");

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn disk_is_non_empty_returns_true_for_mbr_disk() {
        // ARRANGE
        let disk = NamedTempFile::new().expect("temp file");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        file.set_len(4096).expect("resize");
        mbr::io::write_entry(
            &mut file,
            0,
            &MbrPartitionEntry {
                bootable: false,
                partition_type: 0x83,
                starting_lba: 1,
                size_lba: 1,
            },
        )
        .expect("write mbr entry");
        mbr::io::write_signature(&mut file).expect("write mbr signature");

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(result);
    }
}
