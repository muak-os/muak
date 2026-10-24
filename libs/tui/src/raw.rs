//! Raw terminal mode helpers.

use std::io;
use std::os::fd::AsFd;

use rustix::termios::{
    ControlModes, InputModes, LocalModes, OptionalActions, OutputModes, SpecialCodeIndex, Termios,
    tcgetattr, tcsetattr,
};

/// Saved terminal attributes for restoring raw mode.
pub struct Original {
    termios: Termios,
}

/// Captures the current terminal attributes.
///
/// # Errors
///
/// Returns an error when the terminal attributes cannot be read.
pub fn capture<Fd: AsFd>(fd: Fd) -> io::Result<Original> {
    let termios = tcgetattr(fd).map_err(io::Error::from)?;

    Ok(Original { termios })
}

/// Enables raw mode and returns the previous attributes.
///
/// # Errors
///
/// Returns an error when the terminal attributes cannot be read or applied.
pub fn enable<Fd: AsFd>(fd: Fd) -> io::Result<Original> {
    let original = capture(&fd)?;
    let mut raw = original.termios.clone();
    make_raw(&mut raw);
    tcsetattr(fd, OptionalActions::Flush, &raw).map_err(io::Error::from)?;

    Ok(original)
}

/// Restores previously captured terminal attributes.
///
/// # Errors
///
/// Returns an error when the terminal attributes cannot be applied.
pub fn restore<Fd: AsFd>(fd: Fd, original: &Original) -> io::Result<()> {
    tcsetattr(fd, OptionalActions::Flush, &original.termios).map_err(io::Error::from)
}

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
