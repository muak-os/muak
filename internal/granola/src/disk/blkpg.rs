use crate::log;
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

pub fn add_partition_blkpg(
    disk: &str,
    partition_num: u32,
    start_lba: u64,
    end_lba: u64,
) -> Result<()> {
    log!(
        "installer",
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
        op: 1, // add
        flags: 0,
        datalen: std::mem::size_of::<BlkpgPartition>() as i32,
        data: &mut blkpg_part as *mut BlkpgPartition,
    };

    match unsafe { blkpg_ioctl(f.as_raw_fd(), &blkpg_arg) } {
        Ok(_) => {
            log!(
                "installer",
                "BLKPG: Successfully added partition {}",
                partition_num
            );
            drop(f);
            Ok(())
        }
        Err(e) => {
            log!(
                "installer",
                "BLKPG: Failed to add partition {}: {}",
                partition_num,
                e
            );
            drop(f);
            bail!("BLKPG ioctl failed for partition {}: {}", partition_num, e)
        }
    }
}
