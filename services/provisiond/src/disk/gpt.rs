//! GPT partition table management and manipulation.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use parttable::gpt::table::Table;
use parttable::gpt::types::{
    ALIGN_1_MIB_SECTORS, EFI_GUID, LINUX_FS_GUID, PlacementRequest, Size, Slot, Start,
};
use parttable::mbr::types::{
    MBR_BOOT_SIGNATURE, MBR_BYTES, MBR_ENTRY_SIZE, MBR_PARTITION_ENTRY_OFFSET,
};

use super::blkpg::{add_partition_blkpg, delete_partition_blkpg};
use super::constants::{EFI_SIZE, SECTOR_SIZE, STATE_SIZE};
use super::format::wait_for_device;

const MBR_MAX_SLOTS: usize = 4;
const MBR_PARTITION_TYPE_OFFSET: usize = 4;

/// Formats a partition device path based on disk naming convention.
pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{}p{}", disk, partition)
    } else {
        format!("{}{}", disk, partition)
    }
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

    let sector_count = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
    let mut gpt = Table::create(sector_count, SECTOR_SIZE, [0xff; 16])?;

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

    f.seek(std::io::SeekFrom::Start(0))?;
    gpt.write_primary_to(sector_count, &mut f)?;
    f.seek(std::io::SeekFrom::Start(
        gpt.backup_data_offset(sector_count),
    ))?;
    gpt.write_backup_to(sector_count, &mut f)?;
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
    let (mut gpt, sector_count, start) = match Table::read(&mut f) {
        Ok(existing) => {
            let sc = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
            kmsg::info!("Appending DATA on existing GPT on {}", disk);
            (existing, sc, Start::AfterLastUsed)
        }
        Err(_) => {
            f.seek(std::io::SeekFrom::Start(0))?;
            let sc = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
            let gpt = Table::create(sc, SECTOR_SIZE, [0xff; 16])?;
            kmsg::info!("Creating new GPT with DATA as partition 1 on {}", disk);
            (gpt, sc, Start::FirstUsable)
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

    f.seek(std::io::SeekFrom::Start(0))?;
    gpt.write_primary_to(sector_count, &mut f)?;
    f.seek(std::io::SeekFrom::Start(
        gpt.backup_data_offset(sector_count),
    ))?;
    gpt.write_backup_to(sector_count, &mut f)?;
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

    let sc = f
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0)
        .checked_div(SECTOR_SIZE)
        .unwrap_or(0);
    f.seek(std::io::SeekFrom::Start(0))?;
    gpt.write_primary_to(sc, &mut f)?;
    f.seek(std::io::SeekFrom::Start(gpt.backup_data_offset(sc)))?;
    gpt.write_backup_to(sc, &mut f)?;
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

    use parttable::gpt::table::Table;
    use parttable::gpt::types::{LINUX_FS_GUID, Partition};
    use parttable::mbr;
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
        let mut gpt = Table::create(
            u64::try_from(f.metadata().expect("metadata").len()).unwrap_or(0) / 512,
            512,
            [0xff; 16],
        )
        .expect("new gpt");
        for (i, &name) in names.iter().enumerate() {
            let mut guid = [0u8; 16];
            guid[0] = i as u8 + 1;
            gpt.set_partition(
                i as u32 + 1,
                Partition {
                    type_guid: LINUX_FS_GUID,
                    unique_guid: guid,
                    starting_lba: 2048 + i as u64 * 4096,
                    ending_lba: 2048 + i as u64 * 4096 + 4095,
                    attributes: 0,
                    name: name.into(),
                },
            );
        }
        let sc = u64::try_from(f.metadata().expect("metadata").len()).unwrap_or(0) / 512;
        gpt.write_primary_to(sc, &mut f).expect("write primary");
        gpt.write_backup_to(sc, &mut f).expect("write backup");
        disk
    }

    fn disk_with_contents(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("temp file");
        f.write_all(bytes).expect("write");
        f
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
        let result = mbr::io::protective_size_lba(disk_size, SECTOR_SIZE);

        // ASSERT
        assert_eq!(result, u32::MAX);
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
        use parttable::mbr::types::MbrPartitionEntry;

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
