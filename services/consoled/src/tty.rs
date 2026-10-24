//! TTY device access for rendering to a virtual console.

use std::fs::File;
use std::io::{self, BufWriter};
use std::os::fd::AsFd as _;

use anyhow::{Context as _, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use tui::raw::{self, Original};
use tui::screen::{Ansi, Screen as _};
use tui::size::{self, Size};

const TTY_PATH: &str = "/dev/tty0";

/// A raw TTY handle with terminal mode management.
pub struct Tty {
    file: File,
    out: BufWriter<File>,
    original: Original,
}

impl Tty {
    /// Opens the virtual console in raw mode if possible.
    pub fn open() -> Result<Option<Self>> {
        let fd = match open(
            TTY_PATH,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("failed to open {TTY_PATH}"));
            }
        };

        let file = File::from(fd);
        let out = BufWriter::new(
            file.try_clone()
                .context("failed to duplicate tty descriptor for buffered output")?,
        );
        let original = raw::enable(file.as_fd())
            .with_context(|| format!("failed to set terminal attributes for {TTY_PATH}"))?;

        Ok(Some(Self {
            file,
            out,
            original,
        }))
    }

    /// Returns the terminal dimensions.
    pub fn size(&self) -> Size {
        size::of(self.file.as_fd())
    }

    /// Returns a cloned file handle for asynchronous input.
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor cannot be duplicated.
    pub fn file_clone(&self) -> io::Result<File> {
        self.file.try_clone()
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let size = size::of(self.file.as_fd());
        let mut screen = Ansi::new(&mut self.out, size);
        let _show_result = screen.show_cursor();
        let _reset_result = screen.reset_scroll_region();
        let _flush_result = screen.flush();
        let _restore_result = raw::restore(self.file.as_fd(), &self.original);
    }
}

impl io::Write for Tty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
