use anyhow::{Result, bail};
use gptman::{GPT, GPTPartitionEntry};
use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
use std::path::Path;

use super::blkpg::add_partition_blkpg;
use super::constants::{EFI_GUID, EFI_SIZE, LINUX_FS_GUID, SECTOR_SIZE, STATE_SIZE};
use super::utils::{format_partition_name, generate_guid};

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

pub fn create_partitions(disk: &str) -> Result<(String, String, String)> {
    kmsg::info!(@ "installer", "Creating GPT partition table on {}", disk);

    let mut f = OpenOptions::new().read(true).write(true).open(disk)?;

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;

    kmsg::info!(@ "installer", "Disk size: {} GB", disk_size / super::constants::GB);

    let mut gpt = GPT::new_from(&mut f, SECTOR_SIZE, [0xff; 16])?;

    // Calculate partition sizes in sectors
    let efi_sectors = EFI_SIZE / SECTOR_SIZE;
    let state_sectors = STATE_SIZE / SECTOR_SIZE;

    // Get usable LBA range
    let first_usable = gpt.header.first_usable_lba;
    let last_usable = gpt.header.last_usable_lba;

    // 1MiB alignment
    let align_lba: u64 = 2048;
    let align_up = |lba: u64| -> u64 {
        if lba.is_multiple_of(align_lba) {
            lba
        } else {
            lba + (align_lba - (lba % align_lba))
        }
    };

    // Partition 1: EFI
    let efi_start = if first_usable < align_lba {
        align_lba
    } else {
        align_up(first_usable)
    };
    let efi_end = efi_start + efi_sectors - 1;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: generate_guid(),
        starting_lba: efi_start,
        ending_lba: efi_end,
        attribute_bits: 0,
        partition_name: "EFI".into(),
    };

    // Partition 2: STATE
    let state_start = align_up(efi_end + 1);
    let state_end = state_start + state_sectors - 1;

    gpt[2] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: generate_guid(),
        starting_lba: state_start,
        ending_lba: state_end,
        attribute_bits: 0,
        partition_name: "STATE".into(),
    };

    // Partition 3: DATA (rest of disk)
    let data_start = align_up(state_end + 1);
    let data_end = last_usable;

    gpt[3] = GPTPartitionEntry {
        partition_type_guid: LINUX_FS_GUID,
        unique_partition_guid: generate_guid(),
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
            kmsg::info!(@ "installer", "Verified: GPT has {} used partitions", count);
        }
        Err(e) => {
            kmsg::warn!(@ "installer", "Could not verify GPT: {}", e);
        }
    }
    drop(verify_f);

    add_partition_blkpg(disk, 1, efi_start, efi_end)?;
    add_partition_blkpg(disk, 2, state_start, state_end)?;
    add_partition_blkpg(disk, 3, data_start, data_end)?;

    kmsg::info!(@ "installer", "All partitions registered successfully");

    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);
    let data_part = format_partition_name(disk, 3);

    kmsg::info!(
        @ "installer",
        "Waiting for partition device nodes to appear..."
    );

    for i in 0..30 {
        if Path::new(&efi_part).exists() {
            kmsg::info!(
                @ "installer",
                "Partition devices created successfully after {} attempts",
                i + 1
            );
            break;
        }

        if i == 29 {
            bail!("Timeout waiting for partition devices to appear. BLKPG may have failed.");
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok((efi_part, state_part, data_part))
}
