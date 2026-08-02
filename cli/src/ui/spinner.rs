//! Async spinner for single-line animated progress indication.

extern crate alloc;

use alloc::sync::Arc;
use core::time::Duration;
use std::io::{IsTerminal as _, Result as IoResult, Write as _, stdout};

use crossterm::{QueueableCommand as _, cursor, terminal};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::style as theme;

const FRAMES: &[&str] = &[
    "\u{2807}", "\u{280B}", "\u{2819}", "\u{2838}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{280F}",
];
const FRAME_DURATION: Duration = Duration::from_millis(80);

/// An animated spinner that renders on a single line.
///
/// # Usage
///
/// ```ignore
/// let spinner = Spinner::start("Loading...");
/// // ... do work ...
/// spinner.success("Done!");
/// ```
pub struct Spinner {
    tx: watch::Sender<SpinnerState>,
    handle: JoinHandle<()>,
    is_tty: bool,
}

#[derive(Clone)]
enum SpinnerState {
    Running(Arc<str>),
    Success(Arc<str>),
    Fail(Arc<str>),
    Stop,
}

impl Spinner {
    /// Starts a new spinner with the given message.
    pub fn start<S>(message: S) -> Self
    where
        S: Into<String>,
    {
        let is_tty = stdout().is_terminal();
        let msg: Arc<str> = message.into().into();
        let (tx, rx) = watch::channel(SpinnerState::Running(Arc::clone(&msg)));

        let handle = if is_tty {
            tokio::spawn(render_task(rx))
        } else {
            println!("  {msg}");
            spawn_noop()
        };

        Self { tx, handle, is_tty }
    }

    /// Updates the spinner message while it's running.
    pub fn update<S>(&self, message: S)
    where
        S: Into<String>,
    {
        let _send_result = self.tx.send(SpinnerState::Running(message.into().into()));
    }

    /// Stops the spinner with a success checkmark.
    pub async fn success<S>(self, message: S)
    where
        S: Into<String>,
    {
        self.finish(SpinnerState::Success(message.into().into()))
            .await;
    }

    /// Stops the spinner with a failure cross.
    pub async fn fail<S>(self, message: S)
    where
        S: Into<String>,
    {
        self.finish(SpinnerState::Fail(message.into().into())).await;
    }

    /// Stops the spinner without printing a final message.
    pub async fn stop(self) {
        self.finish(SpinnerState::Stop).await;
    }

    async fn finish(self, state: SpinnerState) {
        let _send_result = self.tx.send(state.clone());
        let _join_result = self.handle.await;

        if !self.is_tty {
            // Non-TTY: print the final message.
            print_final(state);
        }
    }
}

/// Prints the final message when running without a TTY.
fn print_final(state: SpinnerState) {
    match state {
        SpinnerState::Success(msg) => println!("  \u{2713} {msg}"),
        SpinnerState::Fail(msg) => eprintln!("  \u{2717} {msg}"),
        SpinnerState::Running(_) | SpinnerState::Stop => {}
    }
}

/// Spawns a no-op task for non-TTY mode.
fn spawn_noop() -> JoinHandle<()> {
    tokio::spawn(async move {})
}

/// Returns the spinner frame at the given index.
fn frame_at(idx: usize) -> &'static str {
    FRAMES.get(idx).copied().unwrap_or_default()
}

/// Spawns the render loop and reports terminal write failures.
async fn render_task(rx: watch::Receiver<SpinnerState>) {
    if let Err(err) = render_loop(rx).await {
        eprintln!("spinner render failed: {err}");
    }
}

/// The render loop that runs in a spawned task.
async fn render_loop(mut rx: watch::Receiver<SpinnerState>) -> IoResult<()> {
    let mut out = stdout();
    let mut frame_idx: usize = 0;

    out.queue(cursor::Hide)?;
    out.flush()?;

    loop {
        let state = rx.borrow_and_update().clone();

        match state {
            SpinnerState::Running(msg) => {
                let frame = frame_at(frame_idx.rem_euclid(FRAMES.len()));
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
                write!(out, "  {frame} {msg}")?;
                out.flush()?;
                frame_idx = frame_idx.saturating_add(1);
            }
            SpinnerState::Success(msg) => {
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
                write!(out, "  {} {msg}", theme::success("\u{2713}"))?;
                out.queue(cursor::Show)?;
                writeln!(out)?;
                out.flush()?;
                return Ok(());
            }
            SpinnerState::Fail(msg) => {
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
                write!(out, "  {} {msg}", theme::error("\u{2717}"))?;
                out.queue(cursor::Show)?;
                writeln!(out)?;
                out.flush()?;
                return Ok(());
            }
            SpinnerState::Stop => {
                out.queue(cursor::MoveToColumn(0))?;
                out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
                out.queue(cursor::Show)?;
                out.flush()?;
                return Ok(());
            }
        }

        // Wait for a state change or the frame duration, whichever comes first.
        let _frame_wait = timeout(FRAME_DURATION, rx.changed()).await;
    }
}
