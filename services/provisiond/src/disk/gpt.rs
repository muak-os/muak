//! GPT partition table management and manipulation.

use std::fs::{File, OpenOptions};
use std::io::Seek;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use parttable::{
    ALIGN_1_MIB_SECTORS, EFI_GUID, LINUX_FS_GUID, PlacementRequest, Size, Slot, Start, Table,
};

use super::blkpg::{add_partition_blkpg, delete_partition_blkpg};
use super::constants::{EFI_SIZE, SECTOR_SIZE, STATE_SIZE};
use super::utils::{format_partition_name, wait_for_device};

/// Returns `true` when `disk` already has a Muak STATE partition installed.
pub fn has_state_partition(disk: &str) -> Result<bool> {
    let mut f = File::open(disk)?;
    match Table::read(&mut f) {
        Ok(gpt) => Ok(gpt
            .used_partitions()
            .into_iter()
            .any(|(_, partition)| partition.name == "STATE")),
        Err(_) => Ok(false),
    }
}

fn open_disk_rw(disk: &str) -> Result<(File, u64)> {
    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;
    let size = f.seek(std::io::SeekFrom::End(0))?;
    Ok((f, size))
}

fn verify_gpt(disk: &str) {
    match OpenOptions::new().read(true).open(disk) {
        Ok(mut f) => match Table::read(&mut f) {
            Ok(gpt) => {
                let count = gpt.used_partitions().len();
                kmsg::info!("Verified: GPT on {} has {} used partitions", disk, count);
            }
            Err(e) => kmsg::warn!("Could not verify GPT on {}: {}", disk, e),
        },
        Err(e) => kmsg::warn!("Could not open {} for GPT verification: {}", disk, e),
    }
}

/// Creates EFI and STATE on the system disk.
pub fn create_system_partitions(disk: &str) -> Result<(String, String)> {
    kmsg::info!("Creating GPT with EFI+STATE on system disk {}", disk);

    let (mut f, disk_size) = open_disk_rw(disk)?;
    kmsg::info!("System disk size: {} GB", disk_size / super::constants::GB);

    let mut gpt = Table::create(&mut f, SECTOR_SIZE, [0xff; 16])?;

    let efi = gpt.place_partition(
        PlacementRequest {
            slot: Slot::Exact(1),
            start: Start::FirstUsable,
            size: Size::Bytes(EFI_SIZE),
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid: EFI_GUID,
            unique_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
            attributes: 0,
            name: "EFI".to_owned(),
        },
        SECTOR_SIZE,
    )?;

    let state = gpt.place_partition(
        PlacementRequest {
            slot: Slot::Exact(2),
            start: Start::AfterPartition(efi.number),
            size: Size::Bytes(STATE_SIZE),
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid: LINUX_FS_GUID,
            unique_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
            attributes: 0,
            name: "STATE".to_owned(),
        },
        SECTOR_SIZE,
    )?;

    gpt.write(&mut f)?;
    parttable::write_gpt_protective_mbr(&mut f, disk_size, SECTOR_SIZE)?;
    f.sync_all()?;
    drop(f);

    verify_gpt(disk);

    add_partition_blkpg(
        disk,
        efi.number,
        efi.partition.starting_lba,
        efi.partition.ending_lba,
    )?;
    add_partition_blkpg(
        disk,
        state.number,
        state.partition.starting_lba,
        state.partition.ending_lba,
    )?;

    kmsg::info!("System partitions registered on {}", disk);

    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);

    wait_for_device(&efi_part)?;

    Ok((efi_part, state_part))
}

/// Creates a DATA partition filling the remaining space on `disk`.
pub fn create_data_partition(disk: &str) -> Result<String> {
    let (mut f, disk_size) = open_disk_rw(disk)?;
    kmsg::info!("Data disk size: {} GB", disk_size / super::constants::GB);

    f.seek(std::io::SeekFrom::Start(0))?;
    let (mut gpt, start) = match Table::read(&mut f) {
        Ok(existing) => {
            kmsg::info!("Appending DATA on existing GPT on {}", disk);
            (existing, Start::AfterLastUsed)
        }
        Err(_) => {
            f.seek(std::io::SeekFrom::Start(0))?;
            let gpt = Table::create(&mut f, SECTOR_SIZE, [0xff; 16])?;
            kmsg::info!("Creating new GPT with DATA as partition 1 on {}", disk);
            (gpt, Start::FirstUsable)
        }
    };

    let data = gpt.place_partition(
        PlacementRequest {
            slot: Slot::Auto,
            start,
            size: Size::FillToLastUsable,
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid: LINUX_FS_GUID,
            unique_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
            attributes: 0,
            name: "DATA".to_owned(),
        },
        SECTOR_SIZE,
    )?;

    gpt.write(&mut f)?;
    parttable::write_gpt_protective_mbr(&mut f, disk_size, SECTOR_SIZE)?;
    f.sync_all()?;
    drop(f);

    verify_gpt(disk);

    add_partition_blkpg(
        disk,
        data.number,
        data.partition.starting_lba,
        data.partition.ending_lba,
    )?;

    kmsg::info!("Data partition {} registered on {}", data.number, disk);

    let data_part = format_partition_name(disk, data.number);
    wait_for_device(&data_part)?;

    Ok(data_part)
}

fn uuid_now() -> uuid::Timestamp {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    uuid::Timestamp::from_unix(uuid::NoContext, dur.as_secs(), dur.subsec_nanos())
}

/// Deletes the specified partitions from the GPT and removes their device nodes from the kernel.
pub fn delete_partitions(disk: &str, partitions: &[u32]) -> Result<()> {
    kmsg::info!("Deleting partitions {:?} from GPT on {}", partitions, disk);

    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;
    let mut gpt = Table::read(&mut f)?;

    for &partition_num in partitions {
        if !gpt.is_partition_used(partition_num) {
            kmsg::warn!("Partition {} is already unused, skipping", partition_num);
            continue;
        }

        gpt.remove_partition(partition_num)?;
        kmsg::info!("Removed partition {} from GPT", partition_num);
    }

    gpt.write(&mut f)?;
    f.sync_all()?;
    drop(f);

    for &partition_num in partitions {
        delete_partition_blkpg(disk, partition_num)?;
    }

    kmsg::info!("Partitions deleted successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use parttable::{Partition, Table};
    use tempfile::NamedTempFile;

    use super::*;

    /// Creates a blank disk image of the given size as a named temp file.
    fn blank_disk(size: u64) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("temp file");
        f.write_all(&vec![0u8; size as usize]).expect("write");
        f
    }

    /// Writes a GPT with the given partition names to a temp disk file.
    fn disk_with_partitions(names: &[&str]) -> NamedTempFile {
        const DISK_SIZE: u64 = 64 * 1024 * 1024;
        let disk = blank_disk(DISK_SIZE);
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        let mut gpt = Table::create(&mut f, 512, [0xff; 16]).expect("new gpt");
        for (i, &name) in names.iter().enumerate() {
            let mut guid = [0u8; 16];
            guid[0] = i as u8 + 1;
            gpt.set_partition(
                i as u32 + 1,
                Partition {
                    type_guid: parttable::LINUX_FS_GUID,
                    unique_guid: guid,
                    starting_lba: 2048 + i as u64 * 4096,
                    ending_lba: 2048 + i as u64 * 4096 + 4095,
                    attributes: 0,
                    name: name.into(),
                },
            );
        }
        gpt.write(&mut f).expect("write gpt");
        disk
    }

    #[test]
    fn has_state_partition_returns_false_for_blank_disk() {
        // ARRANGE
        let disk = blank_disk(64 * 1024 * 1024);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn has_state_partition_returns_false_for_efi_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "EFI-only disk must not be treated as installed");
    }

    #[test]
    fn has_state_partition_returns_true_for_state_partition() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI", "STATE"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(
            result,
            "disk with STATE partition must be detected as installed"
        );
    }

    #[test]
    fn has_state_partition_returns_true_for_state_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["STATE"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn has_state_partition_returns_false_for_unrelated_partitions() {
        // ARRANGE
        let disk = disk_with_partitions(&["BOOT", "ROOT", "SWAP"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "non-Muak partitions must not block installation");
    }

    #[test]
    fn protective_mbr_size_for_large_disk_is_clamped() {
        // ARRANGE
        let disk_size = (u32::MAX as u64 + 100) * SECTOR_SIZE;

        // ACT
        let result = parttable::protective_mbr_size_lba(disk_size, SECTOR_SIZE);

        // ASSERT
        assert_eq!(result, u32::MAX);
    }
}
