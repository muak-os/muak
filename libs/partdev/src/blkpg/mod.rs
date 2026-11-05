//! Kernel partition table synchronization through the BLKPG ioctl.

mod abi;

use core::mem;
use std::path::Path;

use blockdev::device;
use parttable::gpt;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use rustix::ioctl::{Opcode, Setter, ioctl};

use self::abi::{BlkpgIoctlArg, BlkpgPartition};
use crate::error::{PartdevError, Result};

const BLKPG_ADD_PARTITION: i32 = 1;
const BLKPG_DEL_PARTITION: i32 = 2;

const BLKPG: Opcode = 0x1269;

const MAX_PARTITIONS: u32 = 128;

/// Whether a partition was present in the kernel when it was deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The kernel reported and removed the partition.
    Present,
    /// The kernel did not have the partition, so there was nothing to remove.
    Absent,
}

/// Adds partition `number` spanning `start_lba..=end_lba` to the kernel.
///
/// # Errors
///
/// Returns an error when the device cannot be opened or the ioctl fails.
pub fn add(disk: &str, number: u32, start_lba: u64, end_lba: u64) -> Result<()> {
    let file = open(disk, OFlags::RDWR, Mode::empty()).map_err(std::io::Error::from)?;

    let sector_size = gpt::plan::SECTOR_SIZE;
    let start_bytes = start_lba.saturating_mul(sector_size);
    let length_bytes = end_lba
        .saturating_sub(start_lba)
        .saturating_add(1)
        .saturating_mul(sector_size);

    let mut partition = BlkpgPartition {
        start: i64::try_from(start_bytes).unwrap_or(0),
        length: i64::try_from(length_bytes).unwrap_or(0),
        pno: i32::try_from(number).unwrap_or(0_i32),
        devname: [0_u8; 64],
        volname: [0_u8; 64],
    };
    copy_partition_name(&mut partition.devname, disk, number);

    let argument = BlkpgIoctlArg {
        op: BLKPG_ADD_PARTITION,
        flags: 0,
        datalen: i32::try_from(mem::size_of::<BlkpgPartition>()).unwrap_or(0_i32),
        data: &raw mut partition,
    };

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing.
    let setter = unsafe { Setter::<BLKPG, BlkpgIoctlArg>::new(argument) };
    // SAFETY: the file descriptor is valid and the setter carries a fully initialized argument.
    unsafe { ioctl(&file, setter) }.map_err(|errno| PartdevError::Blkpg {
        operation: "add",
        number,
        errno,
    })?;

    Ok(())
}

/// Deletes partition `number` from the kernel.
///
/// # Errors
///
/// Returns an error when the device cannot be opened or a non-`NXIO`/`NOENT`
/// ioctl error occurs.
pub fn delete(disk: &str, number: u32) -> Result<Removed> {
    let file = open(disk, OFlags::RDWR, Mode::empty()).map_err(std::io::Error::from)?;

    let mut partition = BlkpgPartition {
        start: 0,
        length: 0,
        pno: i32::try_from(number).unwrap_or(0_i32),
        devname: [0_u8; 64],
        volname: [0_u8; 64],
    };

    let argument = BlkpgIoctlArg {
        op: BLKPG_DEL_PARTITION,
        flags: 0,
        datalen: i32::try_from(mem::size_of::<BlkpgPartition>()).unwrap_or(0_i32),
        data: &raw mut partition,
    };

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing.
    let setter = unsafe { Setter::<BLKPG, BlkpgIoctlArg>::new(argument) };
    // SAFETY: the file descriptor is valid and the setter carries a fully initialized argument.
    match unsafe { ioctl(&file, setter) } {
        Ok(()) => Ok(Removed::Present),
        Err(Errno::NXIO | Errno::NOENT) => Ok(Removed::Absent),
        Err(errno) => Err(PartdevError::Blkpg {
            operation: "delete",
            number,
            errno,
        }),
    }
}

/// Deletes every partition the kernel reports on `disk`.
///
/// # Errors
///
/// Returns an error when a partition cannot be deleted.
pub fn delete_all(disk: &str) -> Result<Vec<(u32, Removed)>> {
    let mut removed = Vec::new();

    for number in 1..=MAX_PARTITIONS {
        let path = device::partition_path(disk, number);
        if !Path::new(&path).exists() {
            continue;
        }

        removed.push((number, delete(disk, number)?));
    }

    Ok(removed)
}

fn copy_partition_name(destination: &mut [u8; 64], disk: &str, number: u32) {
    let name = device::partition_path(disk.trim_start_matches("/dev/"), number);
    let bytes = name.as_bytes();
    let length = bytes.len().min(63);
    if let (Some(destination), Some(source)) = (destination.get_mut(..length), bytes.get(..length))
    {
        destination.copy_from_slice(source);
    }
}
