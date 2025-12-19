use anyhow::{Result, bail};
use nix::ioctl_write_ptr_bad;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

use super::constants::SECTOR_SIZE;
use super::types::{BlkpgIoctlArg, BlkpgPartition};
use super::utils::format_partition_name;

// BLKPG ioctl number from Linux kernel
// #define BLKPG _IO(0x12,105)
ioctl_write_ptr_bad!(blkpg_ioctl, 0x1269, BlkpgIoctlArg);

const BLKPG_ADD_PARTITION: i32 = 1;
const BLKPG_DEL_PARTITION: i32 = 2;

pub fn delete_partition_blkpg(disk: &str, partition_num: u32) -> Result<()> {
    kmsg::info!(
        @ "installer",
        "Removing partition {} from kernel using BLKPG ioctl",
        partition_num
    );

    let f = OpenOptions::new().read(true).write(true).open(disk)?;

    let devname = [0u8; 64];
    let volname = [0u8; 64];

    let mut blkpg_part = BlkpgPartition {
        start: 0,
        length: 0,
        pno: partition_num as i32,
        devname,
        volname,
    };

    let blkpg_arg = BlkpgIoctlArg {
        op: BLKPG_DEL_PARTITION,
        flags: 0,
        datalen: std::mem::size_of::<BlkpgPartition>() as i32,
        data: &mut blkpg_part as *mut BlkpgPartition,
    };

    match unsafe { blkpg_ioctl(f.as_raw_fd(), &blkpg_arg) } {
        Ok(_) => {
            kmsg::info!(
                @ "installer",
                "BLKPG: Successfully removed partition {}",
                partition_num
            );
        }
        Err(nix::errno::Errno::ENXIO) | Err(nix::errno::Errno::ENOENT) => {
            // Partition doesn't exist in kernel, that's fine
            kmsg::info!(
                @ "installer",
                "BLKPG: Partition {} not present in kernel (OK)",
                partition_num
            );
        }
        Err(e) => {
            kmsg::error!(
                @ "installer",
                "BLKPG: Failed to remove partition {}: {}",
                partition_num,
                e
            );
            bail!(
                "BLKPG ioctl failed to remove partition {}: {}",
                partition_num,
                e
            )
        }
    }

    drop(f);
    Ok(())
}

pub fn delete_all_partitions_blkpg(disk: &str) -> Result<()> {
    kmsg::info!(
        @ "installer",
        "Removing all existing partitions from kernel for {}",
        disk
    );

    for partition_num in 1..=128 {
        let part_path = format_partition_name(disk, partition_num);
        if !std::path::Path::new(&part_path).exists() {
            continue;
        }

        delete_partition_blkpg(disk, partition_num)?;
    }

    Ok(())
}

pub fn add_partition_blkpg(
    disk: &str,
    partition_num: u32,
    start_lba: u64,
    end_lba: u64,
) -> Result<()> {
    kmsg::info!(
        @ "installer",
        "Adding partition {} using BLKPG ioctl (LBA {} to {})",
        partition_num,
        start_lba,
        end_lba
    );

    let f = OpenOptions::new().read(true).write(true).open(disk)?;

    let start_bytes = start_lba * SECTOR_SIZE;
    let length_bytes = (end_lba - start_lba + 1) * SECTOR_SIZE;

    let mut devname = [0u8; 64];
    let volname = [0u8; 64];

    let partition_name = format_partition_name(disk.trim_start_matches("/dev/"), partition_num);
    let partition_name_bytes = partition_name.as_bytes();
    let copy_len = partition_name_bytes.len().min(63);
    devname[..copy_len].copy_from_slice(&partition_name_bytes[..copy_len]);

    let mut blkpg_part = BlkpgPartition {
        start: start_bytes as i64,
        length: length_bytes as i64,
        pno: partition_num as i32,
        devname,
        volname,
    };

    let blkpg_arg = BlkpgIoctlArg {
        op: BLKPG_ADD_PARTITION,
        flags: 0,
        datalen: std::mem::size_of::<BlkpgPartition>() as i32,
        data: &mut blkpg_part as *mut BlkpgPartition,
    };

    match unsafe { blkpg_ioctl(f.as_raw_fd(), &blkpg_arg) } {
        Ok(_) => {
            kmsg::info!(
                @ "installer",
                "BLKPG: Successfully added partition {}",
                partition_num
            );
            drop(f);
            Ok(())
        }
        Err(e) => {
            kmsg::error!(
                @ "installer",
                "BLKPG: Failed to add partition {}: {}",
                partition_num,
                e
            );
            drop(f);
            bail!("BLKPG ioctl failed for partition {}: {}", partition_num, e)
        }
    }
}
