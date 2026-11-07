//! Positioned device I/O that never moves the file cursor.

use std::io::{Error as IoError, ErrorKind, Read, Result, Write};
use std::os::fd::AsFd;

use rustix::fs::fsync;
use rustix::io::{Errno, pread, pwrite};

/// Reads and writes a descriptor at a fixed offset without moving its cursor.
pub struct Positioned<'a, Fd: AsFd> {
    fd: &'a Fd,
    offset: u64,
}

impl<'a, Fd: AsFd> Positioned<'a, Fd> {
    /// Wraps `fd` starting at `offset`.
    #[must_use]
    pub fn at(fd: &'a Fd, offset: u64) -> Self {
        Self { fd, offset }
    }
}

impl<Fd: AsFd> Read for Positioned<'_, Fd> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        read_at(self.fd, self.offset, buf)?;
        self.offset = advance(self.offset, buf.len())?;

        Ok(buf.len())
    }
}

impl<Fd: AsFd> Write for Positioned<'_, Fd> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        write_at(self.fd, self.offset, buf)?;
        self.offset = advance(self.offset, buf.len())?;

        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Reads exactly `buffer.len()` bytes at `offset`.
///
/// # Errors
///
/// Returns an error on I/O failure, when `offset` overflows, or when the device ends early.
pub fn read_at<Fd: AsFd>(fd: &Fd, offset: u64, buffer: &mut [u8]) -> Result<()> {
    let mut position = offset;
    let mut remaining = buffer;
    while !remaining.is_empty() {
        let count = match pread(fd, &mut *remaining, position) {
            Ok(count) => count,
            Err(Errno::INTR) => continue,
            Err(error) => return Err(IoError::from(error)),
        };
        if count == 0 {
            return Err(IoError::from(ErrorKind::UnexpectedEof));
        }
        position = advance(position, count)?;
        remaining = remaining.get_mut(count..).unwrap_or_default();
    }

    Ok(())
}

/// Flushes the descriptor to the device.
///
/// # Errors
///
/// Returns an error when the device rejects the flush.
pub fn sync<Fd: AsFd>(fd: &Fd) -> Result<()> {
    fsync(fd).map_err(IoError::from)
}

/// Writes all of `data` at `offset`. Does not sync.
///
/// # Errors
///
/// Returns an error on I/O failure, when `offset` overflows, or when the device accepts nothing.
pub fn write_at<Fd: AsFd>(fd: &Fd, offset: u64, data: &[u8]) -> Result<()> {
    let mut position = offset;
    let mut remaining = data;
    while !remaining.is_empty() {
        let count = match pwrite(fd, remaining, position) {
            Ok(count) => count,
            Err(Errno::INTR) => continue,
            Err(error) => return Err(IoError::from(error)),
        };
        if count == 0 {
            return Err(IoError::from(ErrorKind::WriteZero));
        }
        position = advance(position, count)?;
        remaining = remaining.get(count..).unwrap_or_default();
    }

    Ok(())
}

fn advance(position: u64, count: usize) -> Result<u64> {
    let count = u64::try_from(count).map_err(|_error| IoError::from(ErrorKind::InvalidInput))?;

    position
        .checked_add(count)
        .ok_or_else(|| IoError::from(ErrorKind::InvalidInput))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_applies_the_checked_offset_add() {
        // ARRANGE
        let start = 100_u64;
        let count = 28_usize;

        // ACT
        let position = advance(start, count);

        // ASSERT
        assert_eq!(position.ok(), Some(128), "offsets add without wrapping");
    }

    #[test]
    fn advance_rejects_an_overflowing_offset() {
        // ARRANGE
        let start = u64::MAX;

        // ACT
        let position = advance(start, 1);

        // ASSERT
        assert_eq!(
            position.map_err(|error| error.kind()),
            Err(ErrorKind::InvalidInput)
        );
    }
}
