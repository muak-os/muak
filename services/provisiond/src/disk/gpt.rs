//! GPT partition table management and manipulation.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use gptman::{GPT, GPTPartitionEntry};
use parttable::{
    ALIGN_1_MIB_SECTORS, EFI_GUID, MBR_BOOT_SIGNATURE, MBR_PROTECTIVE_GPT_TYPE, align_up_lba,
    protective_mbr_size_lba, write_gpt_protective_mbr,
};

use super::blkpg::{add_partition_blkpg, delete_partition_blkpg};
use super::constants::{EFI_SIZE, LINUX_FS_GUID, SECTOR_SIZE, STATE_SIZE};
use super::utils::{format_partition_name, wait_for_device};

/// Returns `true` when `disk` has a valid MBR boot signature with a non-GPT partition type.
fn is_mbr_disk(disk: &str) -> Result<bool> {
    let mut f = File::open(disk)?;
    let mut sector = [0u8; 512];
    f.read_exact(&mut sector)?;

    let boot_sig = u16::from_le_bytes([sector[510], sector[511]]);
    if boot_sig != u16::from_le_bytes(MBR_BOOT_SIGNATURE) {
        return Ok(false);
    }

    // Partition type byte is at offset 446 + 4 = 450 (first entry).
    let part_type = sector[450];
    // 0x00 = empty entry (blank disk), 0xEE = GPT protective MBR — both are fine.
    Ok(part_type != 0x00 && part_type != MBR_PROTECTIVE_GPT_TYPE)
}

/// Returns `true` when `disk` already has a Muak STATE partition installed.
pub fn has_existing_partitions(disk: &str) -> Result<bool> {
    if is_mbr_disk(disk)? {
        bail!(
            "Disk '{}' has an MBR partition table. Only GPT disks are supported. \
             Wipe the disk first or use a different one.",
            disk
        );
    }

    let mut f = File::open(disk)?;
    match GPT::find_from(&mut f) {
        Ok(gpt) => Ok(gpt
            .iter()
            .any(|(_, p)| p.is_used() && p.partition_name.as_str() == "STATE")),
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
        Ok(mut f) => match GPT::find_from(&mut f) {
            Ok(gpt) => {
                let count = gpt.iter().filter(|(_, p)| p.is_used()).count();
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

    let mut gpt = GPT::new_from(&mut f, SECTOR_SIZE, [0xff; 16])?;

    let efi_start = align_up_lba(
        gpt.header.first_usable_lba.max(ALIGN_1_MIB_SECTORS),
        ALIGN_1_MIB_SECTORS,
    );
    let efi_end = efi_start + EFI_SIZE / SECTOR_SIZE - 1;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        starting_lba: efi_start,
        ending_lba: efi_end,
        attribute_bits: 0,
        partition_name: "EFI".into(),
    };

    let state_start = align_up_lba(efi_end + 1, ALIGN_1_MIB_SECTORS);
    let state_end = state_start + STATE_SIZE / SECTOR_SIZE - 1;

    gpt[2] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        starting_lba: state_start,
        ending_lba: state_end,
        attribute_bits: 0,
        partition_name: "STATE".into(),
    };

    gpt.write_into(&mut f)?;
    write_gpt_protective_mbr(&mut f, disk_size, SECTOR_SIZE)?;
    f.sync_all()?;
    drop(f);

    verify_gpt(disk);

    add_partition_blkpg(disk, 1, efi_start, efi_end)?;
    add_partition_blkpg(disk, 2, state_start, state_end)?;

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
    let (mut gpt, data_num, data_start) = match GPT::find_from(&mut f) {
        Ok(existing) => {
            let last_end = existing
                .iter()
                .filter(|(_, p)| p.is_used())
                .map(|(_, p)| p.ending_lba)
                .max()
                .unwrap_or(existing.header.first_usable_lba.saturating_sub(1));
            let next_num = existing
                .iter()
                .filter(|(_, p)| p.is_used())
                .map(|(n, _)| n)
                .max()
                .map_or(1, |n| n + 1);
            let start = align_up_lba(last_end + 1, ALIGN_1_MIB_SECTORS);
            kmsg::info!(
                "Appending DATA as partition {} on existing GPT on {}",
                next_num,
                disk
            );
            (existing, next_num, start)
        }
        Err(_) => {
            f.seek(std::io::SeekFrom::Start(0))?;
            let gpt = GPT::new_from(&mut f, SECTOR_SIZE, [0xff; 16])?;
            let start = align_up_lba(
                gpt.header.first_usable_lba.max(ALIGN_1_MIB_SECTORS),
                ALIGN_1_MIB_SECTORS,
            );
            kmsg::info!("Creating new GPT with DATA as partition 1 on {}", disk);
            (gpt, 1, start)
        }
    };

    let data_end = gpt.header.last_usable_lba;

    gpt[data_num] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        starting_lba: data_start,
        ending_lba: data_end,
        attribute_bits: 0,
        partition_name: "DATA".into(),
    };

    gpt.write_into(&mut f)?;
    write_gpt_protective_mbr(&mut f, disk_size, SECTOR_SIZE)?;
    f.sync_all()?;
    drop(f);

    verify_gpt(disk);

    add_partition_blkpg(disk, data_num, data_start, data_end)?;

    kmsg::info!("Data partition {} registered on {}", data_num, disk);

    let data_part = format_partition_name(disk, data_num);
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
    let mut gpt = GPT::find_from(&mut f)?;

    for &partition_num in partitions {
        if gpt[partition_num].is_unused() {
            kmsg::warn!("Partition {} is already unused, skipping", partition_num);
            continue;
        }

        gpt.remove(partition_num)?;
        kmsg::info!("Removed partition {} from GPT", partition_num);
    }

    gpt.write_into(&mut f)?;
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
        const DISK_SIZE: u64 = 64 * 1024 * 1024; // 64 MiB
        let disk = blank_disk(DISK_SIZE);
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        let mut gpt = GPT::new_from(&mut f, 512, [0xff; 16]).expect("new gpt");
        for (i, &name) in names.iter().enumerate() {
            let mut guid = [0u8; 16];
            guid[0] = i as u8 + 1;
            gpt[i as u32 + 1] = GPTPartitionEntry {
                partition_type_guid: super::super::constants::LINUX_FS_GUID,
                unique_partition_guid: guid,
                starting_lba: 2048 + i as u64 * 4096,
                ending_lba: 2048 + i as u64 * 4096 + 4095,
                attribute_bits: 0,
                partition_name: name.into(),
            };
        }
        gpt.write_into(&mut f).expect("write gpt");
        disk
    }

    #[test]
    fn has_existing_partitions_returns_false_for_blank_disk() {
        // ARRANGE
        let disk = blank_disk(64 * 1024 * 1024);

        // ACT
        let result =
            has_existing_partitions(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn has_existing_partitions_returns_false_for_efi_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI"]);

        // ACT
        let result =
            has_existing_partitions(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "EFI-only disk must not be treated as installed");
    }

    #[test]
    fn has_existing_partitions_returns_true_for_state_partition() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI", "STATE"]);

        // ACT
        let result =
            has_existing_partitions(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(
            result,
            "disk with STATE partition must be detected as installed"
        );
    }

    #[test]
    fn has_existing_partitions_returns_true_for_state_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["STATE"]);

        // ACT
        let result =
            has_existing_partitions(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn has_existing_partitions_returns_false_for_unrelated_partitions() {
        // ARRANGE
        let disk = disk_with_partitions(&["BOOT", "ROOT", "SWAP"]);

        // ACT
        let result =
            has_existing_partitions(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "non-Muak partitions must not block installation");
    }

    #[test]
    fn protective_mbr_size_for_large_disk_is_clamped() {
        // ARRANGE
        let disk_size = (u32::MAX as u64 + 100) * SECTOR_SIZE;

        // ACT
        let result = protective_mbr_size_lba(disk_size, SECTOR_SIZE);

        // ASSERT
        assert_eq!(result, u32::MAX);
    }
}
