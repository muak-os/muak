//! Step line rendering.

use std::io;

use super::state::Active;
use crate::live::Options;
use crate::live::renderer::{
    message_line, spinner_line, status_line, write_active, write_frozen, write_plain,
};
use crate::progress::{self, State};
use crate::screen::Screen;

pub(super) fn draw<S: Screen>(
    screen: &mut S,
    options: Options,
    active: Option<&Active>,
    frame: usize,
) -> io::Result<()> {
    let Some(step) = active else {
        return Ok(());
    };
    let width = usize::from(screen.size().cols);
    let line = match step.progress {
        Some((current, total)) => progress::line(
            &State {
                label: &step.label,
                current,
                total,
            },
            width,
            options.theme,
        ),
        None => spinner_line(&step.label, frame, width, options.theme),
    };

    write_active(screen, &line)
}

pub(super) fn freeze<S: Screen>(
    screen: &mut S,
    options: Options,
    message: &str,
    success: bool,
) -> io::Result<()> {
    let width = usize::from(screen.size().cols);
    let line = status_line(message, success, width, options.theme);

    if options.tty {
        write_frozen(screen, &line)
    } else {
        write_plain(screen, &line)
    }
}

pub(super) fn message<S: Screen>(screen: &mut S, text: &str, width: usize) -> io::Result<()> {
    write_plain(screen, &message_line(text, width))
}
