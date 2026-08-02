//! TTY device access for rendering to a virtual console.

extern crate alloc;

use alloc::sync::Arc;
use std::fs::File;
use std::io;
use std::os::fd::AsFd as _;

use anyhow::{Context as _, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use rustix::termios::{
    ControlModes, InputModes, LocalModes, OptionalActions, OutputModes, SpecialCodeIndex, Termios,
    tcgetattr, tcgetwinsize, tcsetattr,
};

const TTY_PATH: &str = "/dev/tty0";

/// A raw TTY handle with terminal mode management.
pub struct Tty {
    file: Arc<File>,
    original_termios: Termios,
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

        let file: Arc<File> = Arc::new(fd.into());

        let original_termios = tcgetattr(file.as_fd())
            .with_context(|| format!("failed to get terminal attributes for {TTY_PATH}"))?;

        let mut raw = original_termios.clone();
        make_raw(&mut raw);

        tcsetattr(file.as_fd(), OptionalActions::Flush, &raw)
            .with_context(|| format!("failed to set terminal attributes for {TTY_PATH}"))?;

        Ok(Some(Self {
            file,
            original_termios,
        }))
    }

    /// Returns `(cols, rows)` by querying `TIOCGWINSZ` on the TTY fd.
    pub fn size(&self) -> (u16, u16) {
        tcgetwinsize(self.file.as_fd()).map_or((80, 25), |ws| (ws.ws_col, ws.ws_row))
    }

    /// Returns a cloned reference to the underlying file for async I/O.
    pub fn file_arc(&self) -> Arc<File> {
        Arc::clone(&self.file)
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let _restore_result = tcsetattr(
            self.file.as_fd(),
            OptionalActions::Flush,
            &self.original_termios,
        );
    }
}

impl io::Write for Tty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.file).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.file).flush()
    }
}

/// Equivalent of `libc::cfmakeraw()` using `rustix` termios types.
fn make_raw(termios: &mut Termios) {
    termios.input_modes &= !(InputModes::IGNBRK
        | InputModes::BRKINT
        | InputModes::PARMRK
        | InputModes::ISTRIP
        | InputModes::INLCR
        | InputModes::IGNCR
        | InputModes::ICRNL
        | InputModes::IXON);

    termios.output_modes &= !OutputModes::OPOST;

    termios.local_modes &= !(LocalModes::ECHO
        | LocalModes::ECHONL
        | LocalModes::ICANON
        | LocalModes::ISIG
        | LocalModes::IEXTEN);

    termios.control_modes &= !(ControlModes::CSIZE | ControlModes::PARENB);
    termios.control_modes |= ControlModes::CS8;

    termios.special_codes[SpecialCodeIndex::VMIN] = 1;
    termios.special_codes[SpecialCodeIndex::VTIME] = 0;
}
