//! Async spinner for single-line animated progress indication.

use std::io::{IsTerminal, Write, stdout};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{QueueableCommand, cursor, terminal};
use tokio::sync::watch;

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
    handle: tokio::task::JoinHandle<()>,
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
    pub fn start(message: impl Into<String>) -> Self {
        let is_tty = stdout().is_terminal();
        let msg: Arc<str> = message.into().into();
        let (tx, rx) = watch::channel(SpinnerState::Running(msg.clone()));

        let handle = if is_tty {
            tokio::spawn(render_loop(rx))
        } else {
            println!("  {msg}");
            tokio::spawn(async {})
        };

        Self { tx, handle, is_tty }
    }

    /// Updates the spinner message while it's running.
    pub fn update(&self, message: impl Into<String>) {
        let _ = self.tx.send(SpinnerState::Running(message.into().into()));
    }

    /// Stops the spinner with a success checkmark.
    pub async fn success(self, message: impl Into<String>) {
        self.finish(SpinnerState::Success(message.into().into()))
            .await;
    }

    /// Stops the spinner with a failure cross.
    pub async fn fail(self, message: impl Into<String>) {
        self.finish(SpinnerState::Fail(message.into().into())).await;
    }

    /// Stops the spinner without printing a final message.
    pub async fn stop(self) {
        self.finish(SpinnerState::Stop).await;
    }

    async fn finish(self, state: SpinnerState) {
        let _ = self.tx.send(state.clone());
        let _ = self.handle.await;

        if !self.is_tty {
            // Non-TTY: print the final message.
            match state {
                SpinnerState::Success(msg) => println!("  \u{2713} {msg}"),
                SpinnerState::Fail(msg) => eprintln!("  \u{2717} {msg}"),
                _ => {}
            }
        }
    }
}

/// The render loop that runs in a spawned task.
async fn render_loop(mut rx: watch::Receiver<SpinnerState>) {
    let mut out = stdout();
    let mut frame_idx: usize = 0;

    let _ = out.queue(cursor::Hide);
    let _ = out.flush();

    loop {
        let state = rx.borrow_and_update().clone();

        match state {
            SpinnerState::Running(ref msg) => {
                let frame = FRAMES[frame_idx % FRAMES.len()];
                let _ = out.queue(cursor::MoveToColumn(0));
                let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
                let _ = write!(out, "  {frame} {msg}");
                let _ = out.flush();
                frame_idx += 1;
            }
            SpinnerState::Success(msg) => {
                let _ = out.queue(cursor::MoveToColumn(0));
                let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
                let _ = write!(out, "  {} {msg}", theme::success("\u{2713}"));
                let _ = out.queue(cursor::Show);
                let _ = writeln!(out);
                let _ = out.flush();
                return;
            }
            SpinnerState::Fail(msg) => {
                let _ = out.queue(cursor::MoveToColumn(0));
                let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
                let _ = write!(out, "  {} {msg}", theme::error("\u{2717}"));
                let _ = out.queue(cursor::Show);
                let _ = writeln!(out);
                let _ = out.flush();
                return;
            }
            SpinnerState::Stop => {
                let _ = out.queue(cursor::MoveToColumn(0));
                let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
                let _ = out.queue(cursor::Show);
                let _ = out.flush();
                return;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(FRAME_DURATION) => {}
            _ = rx.changed() => {}
        }
    }
}
