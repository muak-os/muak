//! Device access utilities for LUKS2.

use std::io::Error as IoError;
use std::os::fd::{AsFd, OwnedFd};

use rustix::fs::{Mode, OFlags};
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

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

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
    fn open_reports_error_for_missing_path() {
        // ACT
        let result = open("/nonexistent/luks2-device", OFlags::RDONLY);

        // ASSERT
        assert!(matches!(result, Err(Error::Io(error)) if error.kind() == ErrorKind::NotFound));
    }
}
