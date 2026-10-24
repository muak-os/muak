//! Animated single-line spinner.

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};

use crate::live::renderer::{
    FRAME_DURATION, Renderer, message_line, spinner_line, status_line, write_active, write_frozen,
    write_plain,
};
use crate::live::{Options, surface_size};
use crate::screen::{Ansi, Screen};
use crate::style::Stream;

#[derive(Clone)]
enum Command {
    Update(String),
    Success(String),
    Fail(String),
    Stop,
    Tick,
}

/// An animated spinner that renders on a single line.
pub struct Spinner {
    renderer: Renderer<Command>,
}

impl Spinner {
    /// Starts a spinner on `screen`.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn start<S, M>(screen: S, message: M, options: Options) -> io::Result<Self>
    where
        S: Screen + Send + 'static,
        M: Into<String>,
    {
        let renderer = Renderer::start("tui-spinner", Command::Stop, move |rx| {
            run(screen, options, rx)
        })?;

        if let Some(handle) = renderer.handle() {
            handle.send_waiting(Command::Update(message.into()));
        }

        Ok(Self { renderer })
    }

    /// Starts a spinner on standard output.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stdout<M: Into<String>>(message: M) -> io::Result<Self> {
        let stream = Stream::Stdout;
        let screen = Ansi::new(std::io::stdout(), surface_size(stream));

        Self::start(screen, message, Options::detect(stream))
    }

    /// Starts a spinner on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stderr<M: Into<String>>(message: M) -> io::Result<Self> {
        let stream = Stream::Stderr;
        let screen = Ansi::new(std::io::stderr(), surface_size(stream));

        Self::start(screen, message, Options::detect(stream))
    }

    /// Updates the spinner message.
    pub fn update<M: Into<String>>(&self, message: M) {
        if let Some(handle) = self.renderer.handle() {
            handle.send(Command::Update(message.into()));
        }
    }

    /// Stops the spinner with a success checkmark.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn success<M: Into<String>>(self, message: M) -> io::Result<()> {
        self.finish_with(Command::Success(message.into()))
    }

    /// Stops the spinner with a failure cross.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn fail<M: Into<String>>(self, message: M) -> io::Result<()> {
        self.finish_with(Command::Fail(message.into()))
    }

    /// Stops the spinner without a final message.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn stop(self) -> io::Result<()> {
        self.renderer.finish()
    }

    fn finish_with(self, command: Command) -> io::Result<()> {
        if let Some(handle) = self.renderer.handle() {
            handle.send_waiting(command);
        }

        self.renderer.finish()
    }
}

fn run<S: Screen>(mut screen: S, options: Options, rx: &Receiver<Command>) -> io::Result<()> {
    let mut message: Option<String> = None;
    let mut frame = 0_usize;

    loop {
        let command = next_command(rx, options.tty, message.is_some());

        match command {
            Command::Update(text) => {
                update(&mut screen, options, &mut message, &mut frame, text)?;
            }
            Command::Tick => tick(&mut screen, options, message.as_ref(), &mut frame)?,
            Command::Success(text) => {
                finish(&mut screen, options, &text, true)?;

                return Ok(());
            }
            Command::Fail(text) => {
                finish(&mut screen, options, &text, false)?;

                return Ok(());
            }
            Command::Stop => {
                clear(&mut screen, options)?;

                return Ok(());
            }
        }
    }
}

fn update<S: Screen>(
    screen: &mut S,
    options: Options,
    message: &mut Option<String>,
    frame: &mut usize,
    text: String,
) -> io::Result<()> {
    if options.tty {
        draw(screen, options, &text, *frame)?;
        *frame = frame.saturating_add(1);
        *message = Some(text);
    } else if message.is_none() {
        let width = usize::from(screen.size().cols);
        write_plain(screen, &message_line(&text, width))?;
        *message = Some(text);
    } else {
        // Plain output prints the initial message only.
    }

    Ok(())
}

fn tick<S: Screen>(
    screen: &mut S,
    options: Options,
    message: Option<&String>,
    frame: &mut usize,
) -> io::Result<()> {
    let Some(text) = message else {
        return Ok(());
    };

    draw(screen, options, text, *frame)?;
    *frame = frame.saturating_add(1);

    Ok(())
}

fn finish<S: Screen>(
    screen: &mut S,
    options: Options,
    message: &str,
    success: bool,
) -> io::Result<()> {
    let width = usize::from(screen.size().cols);
    let line = status_line(message, success, width, options.theme);

    write_frozen(screen, &line)
}

fn clear<S: Screen>(screen: &mut S, options: Options) -> io::Result<()> {
    if !options.tty {
        return Ok(());
    }

    screen.move_to_column(0)?;
    screen.clear_until_newline()?;
    screen.flush()
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

fn draw<S: Screen>(
    screen: &mut S,
    options: Options,
    message: &str,
    frame: usize,
) -> io::Result<()> {
    let width = usize::from(screen.size().cols);
    let line = spinner_line(message, frame, width, options.theme);

    write_active(screen, &line)
}
