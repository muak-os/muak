//! Zeroing helpers that remove any partition table from a device.

use std::fs::{File, OpenOptions};
use std::io::Write as _;

use blockdev::io;

use crate::error::Result;

const ZERO_CHUNK: usize = 16 * 1024;
const HEAD_BYTES: u64 = 10 * 1024 * 1024;
const TAIL_BYTES: u64 = 1024 * 1024;

/// Zeros the first 10 MiB and the last 1 MiB of `device`.
///
/// # Errors
///
/// Returns an error when the device cannot be opened, sized, written, or synced.
pub fn fast(device: &str) -> Result<()> {
    let file = OpenOptions::new().read(true).write(true).open(device)?;
    let size = blockdev::device::size(&file)?;

    write_zeros(&file, 0, HEAD_BYTES.min(size))?;
    if size > TAIL_BYTES {
        write_zeros(&file, size.saturating_sub(TAIL_BYTES), TAIL_BYTES)?;
    }
    io::sync(&file)?;

    Ok(())
}

/// Zeros the whole of `device`.
///
/// # Errors
///
/// Returns an error when the device cannot be opened, sized, written, or synced.
pub fn zero(device: &str) -> Result<()> {
    let file = OpenOptions::new().read(true).write(true).open(device)?;
    let size = blockdev::device::size(&file)?;

    write_zeros(&file, 0, size)?;
    io::sync(&file)?;

    Ok(())
}

fn write_zeros(file: &File, offset: u64, mut bytes: u64) -> Result<()> {
    let zeros = [0_u8; ZERO_CHUNK];
    let chunk_limit = u64::try_from(ZERO_CHUNK).unwrap_or(0);
    let mut writer = io::Positioned::at(file, offset);

    while bytes > 0 {
        let chunk = usize::try_from(bytes.min(chunk_limit)).unwrap_or(0);
        writer.write_all(zeros.get(..chunk).unwrap_or(&zeros))?;
        bytes = bytes.saturating_sub(u64::try_from(chunk).unwrap_or(0));
    }

    Ok(())
}
