//! GPT partition table management and manipulation.

use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};

use anyhow::Result;
use gptman::{GPT, GPTPartitionEntry};

use super::blkpg::{add_partition_blkpg, delete_partition_blkpg};
use super::constants::{EFI_GUID, EFI_SIZE, LINUX_FS_GUID, SECTOR_SIZE, STATE_SIZE};
use super::utils::{format_partition_name, wait_for_device};

/// Checks if a disk has existing partitions in its GPT.
pub fn has_existing_partitions(disk: &str) -> Result<bool> {
    let mut f = File::open(disk)?;

    match GPT::find_from(&mut f) {
        Ok(gpt) => {
            let count = gpt.iter().count();
            Ok(count > 0)
        }
        Err(_) => Ok(false),
    }
}

/// Writes a protective MBR to prevent legacy tools from corrupting the GPT.
fn write_protective_mbr(f: &mut File, disk_size: u64) -> Result<()> {
    let mut pmbr = [0u8; 512];

    // Boot signature
    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;

    // Partition entry at offset 446
    pmbr[446] = 0x00; // Not bootable
    pmbr[450] = 0xEE; // GPT protective type

    // Starting LBA = 1
    pmbr[454] = 0x01;

    // Size in sectors (total LBAs - 1)
    let total_lbas = disk_size / SECTOR_SIZE;
    let part_size = if total_lbas > 0 { total_lbas - 1 } else { 0 } as u32;
    pmbr[458..462].copy_from_slice(&part_size.to_le_bytes());

    f.seek(std::io::SeekFrom::Start(0))?;
    f.write_all(&pmbr)?;

    Ok(())
}

/// Creates EFI, STATE, and DATA partitions on the specified disk.
pub fn create_partitions(disk: &str) -> Result<(String, String, String)> {
    kmsg::info!("Creating GPT partition table on {}", disk);

    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    kmsg::info!("Disk size: {} GB", disk_size / super::constants::GB);

    let mut gpt = GPT::new_from(&mut f, SECTOR_SIZE, [0xff; 16])?;

    let efi_sectors = EFI_SIZE / SECTOR_SIZE;
    let state_sectors = STATE_SIZE / SECTOR_SIZE;

    let first_usable = gpt.header.first_usable_lba;
    let last_usable = gpt.header.last_usable_lba;

    let align_lba: u64 = 2048;
    let align_up = |lba: u64| -> u64 {
        if lba.is_multiple_of(align_lba) {
            lba
        } else {
            lba + (align_lba - (lba % align_lba))
        }
    };

    let efi_start = if first_usable < align_lba {
        align_lba
    } else {
        align_up(first_usable)
    };
    let efi_end = efi_start + efi_sectors - 1;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: *uuid::Uuid::now_v7().as_bytes(),
        starting_lba: efi_start,
        ending_lba: efi_end,
        attribute_bits: 0,
        partition_name: "EFI".into(),
    };

    let state_start = align_up(efi_end + 1);
    let state_end = state_start + state_sectors - 1;

    gpt[2] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: *uuid::Uuid::now_v7().as_bytes(),
        starting_lba: state_start,
        ending_lba: state_end,
        attribute_bits: 0,
        partition_name: "STATE".into(),
    };

    let data_start = align_up(state_end + 1);
    let data_end = last_usable;

    gpt[3] = GPTPartitionEntry {
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

    let mut verify_f = OpenOptions::new().read(true).open(disk)?;
    match GPT::find_from(&mut verify_f) {
        Ok(verify_gpt) => {
            let count = verify_gpt.iter().filter(|(_, p)| p.is_used()).count();
            kmsg::info!("Verified: GPT has {} used partitions", count);
        }
        Err(e) => {
            kmsg::warn!("Could not verify GPT: {}", e);
        }
    }
    drop(verify_f);

    add_partition_blkpg(disk, 1, efi_start, efi_end)?;
    add_partition_blkpg(disk, 2, state_start, state_end)?;
    add_partition_blkpg(disk, 3, data_start, data_end)?;

    kmsg::info!("All partitions registered successfully");

    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);
    let data_part = format_partition_name(disk, 3);

    wait_for_device(&efi_part)?;

    Ok((efi_part, state_part, data_part))
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
