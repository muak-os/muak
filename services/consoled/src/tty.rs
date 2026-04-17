//! TTY device access for rendering to a Linux virtual console.

use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::termios::{
    ControlModes, InputModes, LocalModes, OptionalActions, OutputModes, SpecialCodeIndex, Termios,
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
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => {
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("failed to open {TTY_PATH}"));
            }
        };

        let file: Arc<File> = Arc::new(fd.into());

        let original_termios = rustix::termios::tcgetattr(file.as_fd())
            .with_context(|| format!("failed to get terminal attributes for {TTY_PATH}"))?;

        let mut raw = original_termios.clone();
        make_raw(&mut raw);

        rustix::termios::tcsetattr(file.as_fd(), OptionalActions::Flush, &raw)
            .with_context(|| format!("failed to set terminal attributes for {TTY_PATH}"))?;

        Ok(Some(Self {
            file,
            original_termios,
        }))
    }

    /// Returns `(cols, rows)` by querying `TIOCGWINSZ` on the TTY fd.
    pub fn size(&self) -> (u16, u16) {
        rustix::termios::tcgetwinsize(self.file.as_fd())
            .map(|ws| (ws.ws_col, ws.ws_row))
            .unwrap_or((80, 25))
    }

    /// Returns a cloned reference to the underlying file for async I/O.
    pub fn file_arc(&self) -> Arc<File> {
        Arc::clone(&self.file)
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(
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

/// Equivalent of libc cfmakeraw() using rustix termios types.
fn make_raw(t: &mut Termios) {
    t.input_modes &= !(InputModes::IGNBRK
        | InputModes::BRKINT
        | InputModes::PARMRK
        | InputModes::ISTRIP
        | InputModes::INLCR
        | InputModes::IGNCR
        | InputModes::ICRNL
        | InputModes::IXON);

    t.output_modes &= !OutputModes::OPOST;

    t.local_modes &= !(LocalModes::ECHO
        | LocalModes::ECHONL
        | LocalModes::ICANON
        | LocalModes::ISIG
        | LocalModes::IEXTEN);

    t.control_modes &= !(ControlModes::CSIZE | ControlModes::PARENB);
    t.control_modes |= ControlModes::CS8;

    t.special_codes[SpecialCodeIndex::VMIN] = 1;
    t.special_codes[SpecialCodeIndex::VTIME] = 0;
}
