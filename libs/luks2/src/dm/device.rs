//! Device access utilities for LUKS2.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

use rustix::fs::{Mode, OFlags, open};
use rustix::io::Result as RustixResult;
use rustix::ioctl::{Updater, ioctl};

use super::abi::{BLKPBSZGET_OPCODE, DEFAULT_SECTOR_SIZE};
use crate::error::Result;

pub(crate) fn detect_sector_size(device: &str) -> u32 {
    let Ok(fd) = open(device, OFlags::RDONLY, Mode::empty()) else {
        return DEFAULT_SECTOR_SIZE;
    };

    let mut size = 0_u32;
    // SAFETY: `size` is a valid writable `u32` output buffer for `BLKPBSZGET`.
    let sector_size_updater = unsafe { Updater::<BLKPBSZGET_OPCODE, u32>::new(&mut size) };
    // SAFETY: The updater references a live `u32` buffer for the duration of the call.
    let result = unsafe { ioctl(&fd, sector_size_updater) };

    normalize_sector_size(result, size)
}

pub(crate) fn read_at(device: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(device)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0_u8; len];
    file.read_exact(&mut buffer)?;

    Ok(buffer)
}

pub(crate) fn write_at(device: &str, offset: u64, data: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).open(device)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)?;
    file.sync_all()?;

    Ok(())
}

fn normalize_sector_size(result: RustixResult<()>, size: u32) -> u32 {
    match result {
        Ok(()) if size >= 512 && size.is_power_of_two() => size,
        _ => DEFAULT_SECTOR_SIZE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_sector_size_returns_default_for_regular_file() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().unwrap();

        // ACT
        let sector_size = detect_sector_size(file.path().to_str().unwrap());

        // ASSERT
        assert_eq!(sector_size, DEFAULT_SECTOR_SIZE);
    }

    #[test]
    fn read_and_write_device_round_trip() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(64).unwrap();
        let path = file.path().to_str().unwrap();
        let data = b"luks2-data";

        // ACT
        write_at(path, 8, data).unwrap();
        let read_back = read_at(path, 8, data.len()).unwrap();

        // ASSERT
        assert_eq!(read_back, data);
    }

    #[test]
    fn read_device_reports_short_reads() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(8).unwrap();
        let path = file.path().to_str().unwrap();

        // ACT
        let result = read_at(path, 0, 16);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_device_reports_error_for_missing_path() {
        // ACT
        let result = write_at("/nonexistent/luks2-device", 0, b"data");

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn read_device_reports_error_for_missing_path() {
        // ACT
        let result = read_at("/nonexistent/luks2-device", 0, 4);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn normalize_sector_size_returns_default_for_invalid_values() {
        // ACT
        let result = normalize_sector_size(Ok(()), 123);

        // ASSERT
        assert_eq!(result, DEFAULT_SECTOR_SIZE);
    }

    #[test]
    fn normalize_sector_size_returns_size_for_valid_values() {
        // ACT
        let result = normalize_sector_size(Ok(()), 4096);

        // ASSERT
        assert_eq!(result, 4096);
    }
}
