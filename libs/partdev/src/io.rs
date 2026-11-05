//! Positioned file I/O that never moves the file cursor.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::FileExt as _;

/// Reads and writes a file at a fixed offset without moving the file cursor.
pub(crate) struct Positioned<'a> {
    file: &'a File,
    offset: u64,
}

impl<'a> Positioned<'a> {
    /// Wraps `file` starting at `offset`.
    pub(crate) fn at(file: &'a File, offset: u64) -> Self {
        Self { file, offset }
    }
}

impl Read for Positioned<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read_at(buf, self.offset)?;
        self.offset = self.offset.saturating_add(u64::try_from(read).unwrap_or(0));

        Ok(read)
    }
}

impl Write for Positioned<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write_at(buf, self.offset)?;
        self.offset = self
            .offset
            .saturating_add(u64::try_from(written).unwrap_or(0));

        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
