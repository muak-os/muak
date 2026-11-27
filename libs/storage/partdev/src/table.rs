//! Reading and writing the GPT that lives on a device.

use std::fs::File;

use blockdev::io;
use parttable::gpt;
use parttable::gpt::table::Table;

use crate::error::{PartdevError, Result};
use crate::kernel::Kernel;

/// Reads the primary GPT from `file` without moving its cursor.
///
/// # Errors
///
/// Returns an error when the stream is truncated or the header or entries are corrupt.
pub fn read(file: &File) -> Result<Table> {
    let mut positioned = io::Positioned::at(file, 0);

    Ok(gpt::io::read(&mut positioned)?)
}

/// Writes the primary and backup GPT to `file`, then flushes it to disk.
///
/// # Errors
///
/// Returns an error when `sector_count` is zero, when writing fails, or when
/// the device cannot be synced.
pub fn commit(file: &File, table: &Table, sector_count: u64) -> Result<()> {
    if sector_count == 0 {
        return Err(PartdevError::UnknownSectorCount);
    }

    gpt::io::write_primary(table, sector_count, &mut io::Positioned::at(file, 0))?;
    gpt::io::write_backup(
        table,
        sector_count,
        &mut io::Positioned::at(file, table.backup_data_offset(sector_count)),
    )?;
    io::sync(file)?;

    Ok(())
}

/// Returns the number of 512-byte sectors on the device behind `file`.
///
/// # Errors
///
/// Returns an error when the device size cannot be read.
pub fn sector_count(file: &File) -> Result<u64> {
    sector_count_in(&crate::handle(), file)
}

/// Returns the number of 512-byte sectors on the device behind `file`.
///
/// # Errors
///
/// Returns an error when the device size cannot be read.
pub fn sector_count_in<K: Kernel>(kernel: &K, file: &File) -> Result<u64> {
    let bytes = kernel.size(file)?;

    Ok(bytes.checked_div(gpt::plan::SECTOR_SIZE).unwrap_or(0))
}
