//! Ordered multi-step progress display.

mod progress;
mod render;
mod state;

use std::io;

use state::{Command, run};

use crate::live::Options;
use crate::live::renderer::{Handle, Renderer};
use crate::output::{Flags, Output};
use crate::progress::Unit;
use crate::screen::{Ansi, Screen};
use crate::style::Stream;

/// Cloneable handle for updating a running [`Steps`].
#[derive(Clone)]
pub struct StepsHandle {
    handle: Handle<Command>,
}

/// Ordered multi-step progress display.
pub struct Steps {
    renderer: Renderer<Command>,
    handle: StepsHandle,
}

impl Steps {
    /// Starts a step display on `screen`.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn new<S: Screen + Send + 'static>(screen: S, options: Options) -> io::Result<Self> {
        let renderer = Renderer::start("tui-steps", Command::Stop, move |rx| {
            run(screen, options, rx)
        })?;
        let handle = renderer
            .handle()
            .ok_or_else(|| io::Error::other("renderer thread is not running"))?;

        Ok(Self {
            renderer,
            handle: StepsHandle { handle },
        })
    }

    /// Starts a step display on standard output.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stdout() -> io::Result<Self> {
        Self::stdout_with(Output::detect(Stream::Stdout, Flags::default()))
    }

    /// Starts a step display on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stderr() -> io::Result<Self> {
        Self::stderr_with(Output::detect(Stream::Stderr, Flags::default()))
    }

    /// Starts a step display on standard output with resolved `output`.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stdout_with(output: Output) -> io::Result<Self> {
        let screen = Ansi::new(std::io::stdout(), output.size);

        Self::new(screen, Options::from_output(output))
    }

    /// Starts a step display on standard error with resolved `output`.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub fn stderr_with(output: Output) -> io::Result<Self> {
        let screen = Ansi::new(std::io::stderr(), output.size);

        Self::new(screen, Options::from_output(output))
    }

    /// Returns a cloneable handle for updating steps.
    #[must_use]
    pub fn handle(&self) -> StepsHandle {
        self.handle.clone()
    }

    /// Begins a new step.
    pub fn start<M: Into<String>>(&self, message: M) {
        self.handle.start(message);
    }

    /// Begins a new step with an item progress bar.
    pub fn start_progress<M: Into<String>>(&self, message: M, total: u64) {
        self.handle.start_progress(message, total);
    }

    /// Begins a new step with an item progress bar.
    pub fn start_items<M: Into<String>>(&self, message: M, total: u64) {
        self.handle.start_items(message, total);
    }

    /// Begins a new step with a byte progress bar.
    pub fn start_bytes<M: Into<String>>(&self, message: M, total: u64) {
        self.handle.start_bytes(message, total);
    }

    /// Freezes a note above the active step.
    pub fn note<M: Into<String>>(&self, message: M) {
        self.handle.note(message);
    }

    /// Updates the active step label.
    pub fn set_label<M: Into<String>>(&self, message: M) {
        self.handle.set_label(message);
    }

    /// Updates the active progress value.
    pub fn progress(&self, current: u64) {
        self.handle.progress(current);
    }

    /// Freezes the active step as a success.
    pub fn complete<M: Into<String>>(&self, message: M) {
        self.handle.complete(message);
    }

    /// Freezes the active step as a failure.
    pub fn fail<M: Into<String>>(&self, message: M) {
        self.handle.fail(message);
    }

    /// Freezes the active step and stops the renderer.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread fails.
    pub fn finish(self) -> io::Result<()> {
        self.renderer.finish()
    }
}

impl StepsHandle {
    /// Begins a new step.
    pub fn start<M: Into<String>>(&self, message: M) {
        self.handle.send_waiting(Command::Start(message.into()));
    }

    /// Begins a new step with an item progress bar.
    pub fn start_progress<M: Into<String>>(&self, message: M, total: u64) {
        self.start_items(message, total);
    }

    /// Begins a new step with an item progress bar.
    pub fn start_items<M: Into<String>>(&self, message: M, total: u64) {
        self.handle.send_waiting(Command::StartProgress {
            message: message.into(),
            total,
            unit: Unit::Items,
        });
    }

    /// Begins a new step with a byte progress bar.
    pub fn start_bytes<M: Into<String>>(&self, message: M, total: u64) {
        self.handle.send_waiting(Command::StartProgress {
            message: message.into(),
            total,
            unit: Unit::Bytes,
        });
    }

    /// Freezes a note above the active step.
    pub fn note<M: Into<String>>(&self, message: M) {
        self.handle.send(Command::Note(message.into()));
    }

    /// Updates the active step label.
    pub fn set_label<M: Into<String>>(&self, message: M) {
        self.handle.send(Command::SetLabel(message.into()));
    }

    /// Updates the active progress value.
    pub fn progress(&self, current: u64) {
        self.handle.send(Command::Progress(current));
    }

    /// Freezes the active step as a success.
    pub fn complete<M: Into<String>>(&self, message: M) {
        self.handle.send_waiting(Command::Complete(message.into()));
    }

    /// Freezes the active step as a failure.
    pub fn fail<M: Into<String>>(&self, message: M) {
        self.handle.send_waiting(Command::Fail(message.into()));
    }
}
