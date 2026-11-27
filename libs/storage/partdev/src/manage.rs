//! Device-level partition table queries and deletion.

use std::fs::{File, OpenOptions};

use blockdev::io;
use parttable::mbr;

use crate::error::Result;
use crate::kernel::Kernel;
use crate::table;

/// Returns `true` when `device` carries a GPT or a non-empty MBR.
///
/// # Errors
///
/// Returns an error when the device cannot be opened.
pub fn is_non_empty(device: &str) -> Result<bool> {
    let file = File::open(device)?;
    if table::read(&file).is_ok() {
        return Ok(true);
    }

    match mbr::read(&mut io::Positioned::at(&file, 0)) {
        Ok(entries) => Ok(entries.iter().any(Option::is_some)),
        Err(_) => Ok(false),
    }
}

/// Returns the partition number of the partition named `name` on `device`.
///
/// # Errors
///
/// Returns an error when the device cannot be opened.
pub fn partition_number(device: &str, name: &str) -> Result<Option<u32>> {
    let file = File::open(device)?;
    let Ok(gpt) = table::read(&file) else {
        return Ok(None);
    };

    Ok(gpt
        .partitions()
        .find(|entry| entry.1.name == name)
        .map(|(number, _)| number))
}

/// Deletes `numbers` from the GPT and asks the kernel to re-read it.
///
/// # Errors
///
/// Returns an error when the device cannot be opened, the table cannot be read
/// or written, or the kernel rejects the re-read.
pub fn delete_partitions(disk: &str, numbers: &[u32]) -> Result<()> {
    delete_partitions_in(&crate::handle(), disk, numbers)
}

/// Deletes `numbers` from the GPT through `kernel` and re-reads the table.
///
/// # Errors
///
/// Returns an error when the device cannot be opened, the table cannot be read
/// or written, or the kernel rejects the re-read.
pub fn delete_partitions_in<K: Kernel>(kernel: &K, disk: &str, numbers: &[u32]) -> Result<()> {
    let file = OpenOptions::new().read(true).write(true).open(disk)?;
    let mut gpt = table::read(&file)?;

    for &number in numbers {
        if !gpt.is_partition_used(number) {
            continue;
        }

        gpt.remove_partition(number)?;
    }

    let sectors = table::sector_count_in(kernel, &file)?;
    table::commit(&file, &gpt, sectors)?;
    drop(file);

    kernel.rescan(disk)
}
