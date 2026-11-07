//! Device access utilities for LUKS2.

use std::io::{Error as IoError, ErrorKind};
use std::os::fd::{AsFd, OwnedFd};

use rustix::fs::{Mode, OFlags, fsync};
use rustix::io::{pread, pwrite};
use rustix::ioctl::{Updater, ioctl};

use super::abi::{BLKPBSZGET_OPCODE, DEFAULT_SECTOR_SIZE};
use crate::error::{Luks2Error as Error, Result};

pub(crate) fn open(device: &str, flags: OFlags) -> Result<OwnedFd> {
    rustix::fs::open(device, flags | OFlags::CLOEXEC, Mode::empty())
        .map_err(|error| Error::Io(IoError::from(error)))
}

pub(crate) fn detect_sector_size<Fd: AsFd>(fd: &Fd) -> u32 {
    let mut size = 0_u32;
    // SAFETY: `size` is a valid writable `u32` output buffer for `BLKPBSZGET`.
    let sector_size_updater = unsafe { Updater::<BLKPBSZGET_OPCODE, u32>::new(&mut size) };
    // SAFETY: The updater references a live `u32` buffer for the duration of the call.
    let result = unsafe { ioctl(fd, sector_size_updater) };

    match result {
        Ok(()) if size >= 512 && size.is_power_of_two() => size,
        _ => DEFAULT_SECTOR_SIZE,
    }
}

pub(crate) fn read_at<Fd: AsFd>(fd: &Fd, offset: u64, buffer: &mut [u8]) -> Result<()> {
    let mut position = offset;
    let mut remaining = buffer;
    while !remaining.is_empty() {
        let count = match pread(fd, &mut *remaining, position) {
            Ok(count) => count,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(Error::Io(IoError::from(error))),
        };
        if count == 0 {
            return Err(Error::Io(IoError::from(ErrorKind::UnexpectedEof)));
        }
        position = advance(position, count)?;
        remaining = remaining.get_mut(count..).unwrap_or_default();
    }

    Ok(())
}

pub(crate) fn write_at<Fd: AsFd>(fd: &Fd, offset: u64, data: &[u8]) -> Result<()> {
    let mut position = offset;
    let mut remaining = data;
    while !remaining.is_empty() {
        let count = match pwrite(fd, remaining, position) {
            Ok(count) => count,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(Error::Io(IoError::from(error))),
        };
        if count == 0 {
            return Err(Error::Io(IoError::from(ErrorKind::WriteZero)));
        }
        position = advance(position, count)?;
        remaining = remaining.get(count..).unwrap_or_default();
    }

    fsync(fd).map_err(IoError::from)?;

    Ok(())
}

fn advance(position: u64, count: usize) -> Result<u64> {
    let count = u64::try_from(count)
        .map_err(|_error| Error::InvalidField("device I/O count overflow".into()))?;

    position
        .checked_add(count)
        .ok_or_else(|| Error::InvalidField("device offset overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_sector_size_returns_default_for_regular_file() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().unwrap();
        let fd = open(file.path().to_str().unwrap(), OFlags::WRONLY).unwrap();

        // ACT
        let sector_size = detect_sector_size(&fd);

        // ASSERT
        assert_eq!(sector_size, DEFAULT_SECTOR_SIZE);
    }

    #[test]
    fn read_and_write_device_round_trip() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(64).unwrap();
        let fd = open(file.path().to_str().unwrap(), OFlags::RDWR).unwrap();
        let data = b"luks2-data";

        // ACT
        write_at(&fd, 8, data).unwrap();
        let mut read_back = [0_u8; 10];
        read_at(&fd, 8, &mut read_back).unwrap();

        // ASSERT
        assert_eq!(&read_back, data);
    }

    #[test]
    fn read_device_reports_short_reads() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(8).unwrap();
        let fd = open(file.path().to_str().unwrap(), OFlags::RDONLY).unwrap();
        let mut buffer = [0_u8; 16];

        // ACT
        let result = read_at(&fd, 0, &mut buffer);

        // ASSERT
        assert!(
            matches!(result, Err(Error::Io(error)) if error.kind() == ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn read_device_accepts_empty_buffer() {
        // ARRANGE
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(8).unwrap();
        let fd = open(file.path().to_str().unwrap(), OFlags::RDONLY).unwrap();

        // ACT
        let result = read_at(&fd, 0, &mut []);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn open_reports_error_for_missing_path() {
        // ACT
        let result = open("/nonexistent/luks2-device", OFlags::RDONLY);

        // ASSERT
        assert!(matches!(result, Err(Error::Io(error)) if error.kind() == ErrorKind::NotFound));
    }
}
