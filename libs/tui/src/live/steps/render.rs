//! Step line rendering.

use std::io;

use super::state::Active;
use crate::live::Options;
use crate::live::lines::{message_line, note_line, spinner_line, status_line};
use crate::live::renderer::{write_active, write_frozen, write_plain};
use crate::output::Mode;
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
    let line = match step.progress.as_ref() {
        Some(progress) => progress::line(
            &State {
                label: &step.label,
                current: progress.current,
                total: progress.total,
                unit: progress.unit,
                rate: progress.rate,
                eta: progress.eta,
            },
            width,
            options.theme,
            options.glyphs,
        ),
        None => spinner_line(
            &step.label,
            frame,
            width,
            options.theme,
            options.glyphs,
            Some(step.started.elapsed()),
        ),
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
    let line = status_line(message, success, width, options.theme, options.glyphs);

    if options.tty {
        write_frozen(screen, &line)
    } else {
        write_plain(screen, &line)
    }
}

pub(super) fn message<S: Screen>(screen: &mut S, text: &str, width: usize) -> io::Result<()> {
    write_plain(screen, &message_line(text, width))
}

pub(super) fn note<S: Screen>(
    screen: &mut S,
    options: Options,
    active: Option<&Active>,
    frame: usize,
    message: &str,
) -> io::Result<()> {
    if options.mode == Mode::Quiet {
        return Ok(());
    }

    let width = usize::from(screen.size().cols);
    let line = note_line(message, width, options.theme, options.glyphs);
    let redraw = options.tty && active.is_some();

    if redraw {
        screen.move_to_column(0)?;
        screen.clear_until_newline()?;
    }
    write_plain(screen, &line)?;
    if redraw {
        draw(screen, options, active, frame)?;
    }

    Ok(())
}
