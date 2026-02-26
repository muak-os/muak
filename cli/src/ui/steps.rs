//! Vertical multistep progress indicator.

use std::io::{IsTerminal, Write, stdout};
use std::time::Duration;

use crossterm::{QueueableCommand, cursor, terminal};
use tokio::sync::mpsc;

use super::style as theme;

const FRAMES: &[&str] = &[
    "\u{2807}", "\u{280B}", "\u{2819}", "\u{2838}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{280F}",
];
const FRAME_DURATION: Duration = Duration::from_millis(80);

/// Multi-step progress tracker.
///
/// # Usage
///
/// ```ignore
/// let mut steps = Steps::new();
/// steps.start("Partitioning disk...");
/// // ... work ...
/// steps.start("Writing filesystem...");
/// // ... work ...
/// steps.complete("Installation finished");
/// steps.finish().await;
/// ```
pub struct Steps {
    is_tty: bool,
    tx: Option<mpsc::Sender<StepCmd>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

enum StepCmd {
    Start(String),
    Complete(String),
    Fail(String),
    Stop,
}

impl Default for Steps {
    fn default() -> Self {
        Self::new()
    }
}

impl Steps {
    /// Creates a new multi-step progress tracker.
    pub fn new() -> Self {
        let is_tty = stdout().is_terminal();

        if !is_tty {
            return Self {
                is_tty,
                tx: None,
                handle: None,
            };
        }

        let (tx, rx) = mpsc::channel(16);
        let handle = tokio::spawn(render(rx));

        Self {
            is_tty,
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Begins a new step.
    pub fn start(&self, message: impl Into<String>) {
        if !self.is_tty {
            println!("  {}", message.into());
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(StepCmd::Start(message.into()));
        }
    }

    /// Completes the current step with a specific success message.
    pub fn complete(&self, message: impl Into<String>) {
        if !self.is_tty {
            println!("  \u{2713} {}", message.into());
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(StepCmd::Complete(message.into()));
        }
    }

    /// Fails the current step with an error message.
    pub fn fail(&self, message: impl Into<String>) {
        if !self.is_tty {
            eprintln!("  \u{2717} {}", message.into());
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(StepCmd::Fail(message.into()));
        }
    }

    /// Cleans up resources. Call this when all steps are done.
    pub async fn finish(self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(StepCmd::Stop).await;
        }
        if let Some(handle) = self.handle {
            let _ = handle.await;
        }
    }
}

/// Freezes the current spinner line as a completed step.
fn freeze_success(out: &mut impl Write, msg: &str) {
    let _ = out.queue(cursor::MoveToColumn(0));
    let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
    let _ = write!(out, "  {} {msg}", theme::success("\u{2713}"));
    let _ = writeln!(out);
    let _ = out.flush();
}

/// Freezes the current spinner line as a failed step.
fn freeze_fail(out: &mut impl Write, msg: &str) {
    let _ = out.queue(cursor::MoveToColumn(0));
    let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
    let _ = write!(out, "  {} {msg}", theme::error("\u{2717}"));
    let _ = writeln!(out);
    let _ = out.flush();
}

/// Renders a spinner frame.
fn render_frame(out: &mut impl Write, frame: &str, msg: &str) {
    let _ = out.queue(cursor::MoveToColumn(0));
    let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
    let _ = write!(out, "  {frame} {msg}");
    let _ = out.flush();
}

/// Handles a single command. Returns `true` if the loop should exit.
fn handle_cmd(
    cmd: StepCmd,
    out: &mut impl Write,
    current_msg: &mut Option<String>,
    frame_idx: &mut usize,
) -> bool {
    match cmd {
        StepCmd::Start(msg) => {
            if let Some(prev) = current_msg.take() {
                freeze_success(out, &prev);
            }
            *frame_idx = 0;
            render_frame(out, FRAMES[0], &msg);
            *frame_idx = 1;
            *current_msg = Some(msg);
            false
        }
        StepCmd::Complete(msg) => {
            *current_msg = None;
            freeze_success(out, &msg);
            false
        }
        StepCmd::Fail(msg) => {
            *current_msg = None;
            freeze_fail(out, &msg);
            false
        }
        StepCmd::Stop => {
            if let Some(prev) = current_msg.take() {
                freeze_success(out, &prev);
            }
            true
        }
    }
}

/// The render loop for the steps component.
async fn render(mut rx: mpsc::Receiver<StepCmd>) {
    let mut out = stdout();
    let mut frame_idx: usize = 0;
    let mut current_msg: Option<String> = None;

    let _ = out.queue(cursor::Hide);
    let _ = out.flush();

    loop {
        if current_msg.is_some() {
            match tokio::time::timeout(FRAME_DURATION, rx.recv()).await {
                Ok(Some(cmd)) => {
                    if handle_cmd(cmd, &mut out, &mut current_msg, &mut frame_idx) {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    if let Some(ref msg) = current_msg {
                        let frame = FRAMES[frame_idx % FRAMES.len()];
                        render_frame(&mut out, frame, msg);
                        frame_idx += 1;
                    }
                }
            }
        } else {
            match rx.recv().await {
                Some(cmd) => {
                    if handle_cmd(cmd, &mut out, &mut current_msg, &mut frame_idx) {
                        break;
                    }
                }
                None => break,
            }
        }
    }

    let _ = out.queue(cursor::Show);
    let _ = out.flush();
}
