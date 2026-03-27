//! GPT partition table management and manipulation.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};

use anyhow::{Result, bail};
use gptman::{GPT, GPTPartitionEntry};

use super::blkpg::{add_partition_blkpg, delete_partition_blkpg};
use super::constants::{EFI_GUID, EFI_SIZE, LINUX_FS_GUID, SECTOR_SIZE, STATE_SIZE};
use super::utils::{format_partition_name, wait_for_device};

/// Returns `true` when `disk` has a valid MBR boot signature with a non-GPT partition type.
fn is_mbr_disk(disk: &str) -> Result<bool> {
    let mut f = File::open(disk)?;
    let mut sector = [0u8; 512];
    f.read_exact(&mut sector)?;

    let boot_sig = u16::from_le_bytes([sector[510], sector[511]]);
    if boot_sig != 0xAA55 {
        return Ok(false);
    }

    // Partition type byte is at offset 446 + 4 = 450 (first entry).
    let part_type = sector[450];
    // 0x00 = empty entry (blank disk), 0xEE = GPT protective MBR — both are fine.
    Ok(part_type != 0x00 && part_type != 0xEE)
}

/// Checks if a disk has existing partitions in its GPT.
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
        Ok(gpt) => Ok(gpt.iter().count() > 0),
        Err(_) => Ok(false),
    }
}

/// Writes a protective MBR to prevent legacy tools from corrupting the GPT.
fn write_protective_mbr(f: &mut File, disk_size: u64) -> Result<()> {
    let mut pmbr = [0u8; 512];

    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;
    pmbr[446] = 0x00; // Not bootable
    pmbr[450] = 0xEE; // GPT protective type
    pmbr[454] = 0x01; // Starting LBA = 1

    let total_lbas = disk_size / SECTOR_SIZE;
    let part_size = if total_lbas > 0 { total_lbas - 1 } else { 0 } as u32;
    pmbr[458..462].copy_from_slice(&part_size.to_le_bytes());

    f.seek(std::io::SeekFrom::Start(0))?;
    f.write_all(&pmbr)?;

    Ok(())
}

fn align_up(lba: u64, align: u64) -> u64 {
    if lba.is_multiple_of(align) {
        lba
    } else {
        lba + (align - (lba % align))
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

    const ALIGN: u64 = 2048;
    let efi_start = align_up(gpt.header.first_usable_lba.max(ALIGN), ALIGN);
    let efi_end = efi_start + EFI_SIZE / SECTOR_SIZE - 1;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: *uuid::Uuid::now_v7().as_bytes(),
        starting_lba: efi_start,
        ending_lba: efi_end,
        attribute_bits: 0,
        partition_name: "EFI".into(),
    };

    let state_start = align_up(efi_end + 1, ALIGN);
    let state_end = state_start + STATE_SIZE / SECTOR_SIZE - 1;

    gpt[2] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: *uuid::Uuid::now_v7().as_bytes(),
        starting_lba: state_start,
        ending_lba: state_end,
        attribute_bits: 0,
        partition_name: "STATE".into(),
    };

    gpt.write_into(&mut f)?;
    write_protective_mbr(&mut f, disk_size)?;
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

    const ALIGN: u64 = 2048;

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
            let start = align_up(last_end + 1, ALIGN);
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
            let start = align_up(gpt.header.first_usable_lba.max(ALIGN), ALIGN);
            kmsg::info!("Creating new GPT with DATA as partition 1 on {}", disk);
            (gpt, 1, start)
        }
    };

    let data_end = gpt.header.last_usable_lba;

    gpt[data_num] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: *uuid::Uuid::now_v7().as_bytes(),
        starting_lba: data_start,
        ending_lba: data_end,
        attribute_bits: 0,
        partition_name: "DATA".into(),
    };

    gpt.write_into(&mut f)?;
    write_protective_mbr(&mut f, disk_size)?;
    f.sync_all()?;
    drop(f);

    verify_gpt(disk);

    add_partition_blkpg(disk, data_num, data_start, data_end)?;

    kmsg::info!("Data partition {} registered on {}", data_num, disk);

    let data_part = format_partition_name(disk, data_num);
    wait_for_device(&data_part)?;

    Ok(data_part)
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
    use super::*;

    #[test]
    fn align_up_already_aligned_returns_same_value() {
        // ARRANGE
        let lba = 2048u64;
        let align = 2048u64;

        // ACT
        let result = align_up(lba, align);

        // ASSERT
        assert_eq!(result, 2048);
    }

    #[test]
    fn align_up_rounds_unaligned_lba_to_next_multiple() {
        // ARRANGE
        let lba = 2049u64;
        let align = 2048u64;

        // ACT
        let result = align_up(lba, align);

        // ASSERT
        assert_eq!(result, 4096);
    }

    #[test]
    fn align_up_zero_is_already_aligned() {
        // ARRANGE
        let lba = 0u64;
        let align = 2048u64;

        // ACT
        let result = align_up(lba, align);

        // ASSERT
        assert_eq!(result, 0);
    }

    #[test]
    fn align_up_one_less_than_align_rounds_up_to_align() {
        // ARRANGE
        let lba = 2047u64;
        let align = 2048u64;

        // ACT
        let result = align_up(lba, align);

        // ASSERT
        assert_eq!(result, 2048);
    }

    #[test]
    fn align_up_result_is_always_a_multiple_of_align() {
        // ARRANGE
        let align = 2048u64;

        for lba in [1u64, 100, 2047, 2048, 2049, 4095, 4096, 100_000] {
            // ACT
            let result = align_up(lba, align);

            // ASSERT
            assert_eq!(
                result % align,
                0,
                "align_up({}, {}) = {} is not aligned",
                lba,
                align,
                result
            );
            assert!(result >= lba, "align_up must not reduce lba");
        }
    }
}
