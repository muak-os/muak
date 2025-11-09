use crate::log;
use anyhow::{Result, bail};
use gptman::{GPT, GPTPartitionEntry};
use std::fs::{File, OpenOptions};
use std::io::Seek;
use std::path::Path;

use super::blkpg::add_partition_blkpg;
use super::constants::{EFI_GUID, EFI_SIZE, GB, LINUX_FS_GUID, SECTOR_SIZE, STATE_SIZE};
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

pub fn wipe_disk(disk: &str) -> Result<()> {
    use super::constants::MB;
    use std::io::Write;

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

    let disk_size = f.seek(std::io::SeekFrom::End(0))?;
    f.seek(std::io::SeekFrom::Start(0))?;

    log!("installer", "Disk size: {} GB", disk_size / GB);

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

    gpt.write_into(&mut f)?;
    f.sync_all()?;
    drop(f);

    // Verify GPT was written by reading it back
    let mut verify_f = OpenOptions::new().read(true).open(disk)?;
    match GPT::find_from(&mut verify_f) {
        Ok(verify_gpt) => {
            let count = verify_gpt.iter().filter(|(_, p)| p.is_used()).count();
            log!("installer", "Verified: GPT has {} used partitions", count);
            for (i, partition) in verify_gpt.iter() {
                if partition.is_used() {
                    log!(
                        "installer",
                        "  Partition {}: LBA {} to {}",
                        i,
                        partition.starting_lba,
                        partition.ending_lba
                    );
                }
            }
        }
        Err(e) => {
            log!("installer", "Warning: Could not verify GPT: {}", e);
        }
    }
    drop(verify_f);

    add_partition_blkpg(disk, 1, efi_start, efi_end)?;
    add_partition_blkpg(disk, 2, state_start, state_end)?;
    add_partition_blkpg(disk, 3, data_start, data_end)?;

    log!("installer", "All partitions registered successfully");

    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);
    let data_part = format_partition_name(disk, 3);

    log!(
        "installer",
        "Waiting for partition device nodes to appear..."
    );

    for i in 0..30 {
        let dev_exists = Path::new(&efi_part).exists();

        if dev_exists {
            log!(
                "installer",
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
