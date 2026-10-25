//! Step state machine.

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use super::progress::Progress;
use super::render::{draw, freeze, note};
use crate::live::Options;
use crate::live::renderer::FRAME_DURATION;
use crate::output::Mode;
use crate::progress::Unit;
use crate::screen::Screen;

#[derive(Clone)]
pub(super) enum Command {
    Start(String),
    StartProgress {
        message: String,
        total: u64,
        unit: Unit,
    },
    SetLabel(String),
    Progress(u64),
    Note(String),
    Complete(String),
    Fail(String),
    Stop,
    Tick,
}

pub(super) struct Active {
    pub(super) label: String,
    pub(super) progress: Option<Progress>,
    pub(super) started: Instant,
}

pub(super) fn run<S: Screen>(
    mut screen: S,
    options: Options,
    rx: &Receiver<Command>,
) -> io::Result<()> {
    let mut active: Option<Active> = None;
    let mut frame = 0_usize;

    loop {
        let command = next_command(rx, options.tty, active.is_some());
        if handle(command, &mut screen, options, &mut active, &mut frame)? {
            return Ok(());
        }
    }
}

fn next_command(rx: &Receiver<Command>, tty: bool, animating: bool) -> Command {
    if tty && animating {
        return match rx.recv_timeout(FRAME_DURATION) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => Command::Tick,
            Err(RecvTimeoutError::Disconnected) => Command::Stop,
        };
    }

    rx.recv().unwrap_or(Command::Stop)
}

fn handle<S: Screen>(
    command: Command,
    screen: &mut S,
    options: Options,
    active: &mut Option<Active>,
    frame: &mut usize,
) -> io::Result<bool> {
    match command {
        Command::Start(message) => begin(screen, options, active, frame, &message, None)?,
        Command::StartProgress {
            message,
            total,
            unit,
        } => {
            begin(
                screen,
                options,
                active,
                frame,
                &message,
                Some((total, unit)),
            )?;
        }
        Command::SetLabel(message) => label(screen, options, active, *frame, message)?,
        Command::Progress(current) => progress(screen, options, active, *frame, current)?,
        Command::Note(message) => note(screen, options, active.as_ref(), *frame, &message)?,
        Command::Complete(message) => {
            *active = None;
            if options.mode != Mode::Quiet {
                freeze(screen, options, &message, true)?;
            }
        }
        Command::Fail(message) => {
            *active = None;
            freeze(screen, options, &message, false)?;
        }
        Command::Stop => {
            let step = active.take();
            if options.mode != Mode::Quiet
                && let Some(step) = step
            {
                freeze(screen, options, &step.label, true)?;
            }

            return Ok(true);
        }
        Command::Tick => {
            draw(screen, options, active.as_ref(), *frame)?;
            *frame = frame.saturating_add(1);
        }
    }

    Ok(false)
}

fn label<S: Screen>(
    screen: &mut S,
    options: Options,
    active: &mut Option<Active>,
    frame: usize,
    message: String,
) -> io::Result<()> {
    let Some(step) = active.as_mut() else {
        return Ok(());
    };

    step.label = message;
    if options.tty {
        draw(screen, options, active.as_ref(), frame)?;
    }

    Ok(())
}

fn progress<S: Screen>(
    screen: &mut S,
    options: Options,
    active: &mut Option<Active>,
    frame: usize,
    current: u64,
) -> io::Result<()> {
    let Some(step) = active.as_mut() else {
        return Ok(());
    };
    let Some(value) = step.progress.as_mut() else {
        return Ok(());
    };

    value.set(current);
    if options.tty {
        draw(screen, options, active.as_ref(), frame)?;
    }

    Ok(())
}

fn begin<S: Screen>(
    screen: &mut S,
    options: Options,
    active: &mut Option<Active>,
    frame: &mut usize,
    message: &str,
    progress: Option<(u64, Unit)>,
) -> io::Result<()> {
    if let Some(previous) = active.take() {
        freeze(screen, options, &previous.label, true)?;
    }

    let step = Active {
        label: message.to_owned(),
        progress: progress.map(|(total, unit)| Progress::new(total, unit)),
        started: Instant::now(),
    };
    if options.tty {
        *frame = 0;
        *active = Some(step);
        draw(screen, options, active.as_ref(), *frame)?;
    } else {
        if options.mode == Mode::Plain {
            let width = usize::from(screen.size().cols);
            super::render::message(screen, message, width)?;
        }
        *active = Some(step);
    }

    Ok(())
}
