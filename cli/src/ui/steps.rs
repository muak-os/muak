//! Vertical multistep progress indicator.

use core::time::Duration;
use std::io::{IsTerminal as _, Result as IoResult, Write, stdout};

use crossterm::{QueueableCommand as _, cursor, terminal};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

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
    handle: Option<JoinHandle<()>>,
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
    #[must_use]
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
    pub fn start<S>(&self, message: S)
    where
        S: Into<String>,
    {
        let message = message.into();
        if !self.is_tty {
            println!("  {message}");
            return;
        }
        if let Some(tx) = self.tx.as_ref() {
            let _send_result = tx.try_send(StepCmd::Start(message));
        }
    }

    /// Completes the current step with a specific success message.
    pub fn complete<S>(&self, message: S)
    where
        S: Into<String>,
    {
        let message = message.into();
        if !self.is_tty {
            println!("  \u{2713} {message}");
            return;
        }
        if let Some(tx) = self.tx.as_ref() {
            let _send_result = tx.try_send(StepCmd::Complete(message));
        }
    }

    /// Fails the current step with an error message.
    pub fn fail<S>(&self, message: S)
    where
        S: Into<String>,
    {
        let message = message.into();
        if !self.is_tty {
            eprintln!("  \u{2717} {message}");
            return;
        }
        if let Some(tx) = self.tx.as_ref() {
            let _send_result = tx.try_send(StepCmd::Fail(message));
        }
    }

    /// Cleans up resources. Call this when all steps are done.
    pub async fn finish(self) {
        if let Some(tx) = self.tx.as_ref() {
            let _send_result = tx.send(StepCmd::Stop).await;
        }
        if let Some(handle) = self.handle {
            let _join_result = handle.await;
        }
    }
}

/// Freezes the current spinner line as a completed step.
fn freeze_success(out: &mut impl Write, msg: &str) -> IoResult<()> {
    out.queue(cursor::MoveToColumn(0))?;
    out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
    write!(out, "  {} {msg}", theme::success("\u{2713}"))?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

/// Freezes the current spinner line as a failed step.
fn freeze_fail(out: &mut impl Write, msg: &str) -> IoResult<()> {
    out.queue(cursor::MoveToColumn(0))?;
    out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
    write!(out, "  {} {msg}", theme::error("\u{2717}"))?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

/// Renders a spinner frame.
fn render_frame(out: &mut impl Write, frame: &str, msg: &str) -> IoResult<()> {
    out.queue(cursor::MoveToColumn(0))?;
    out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
    write!(out, "  {frame} {msg}")?;
    out.flush()?;
    Ok(())
}

/// Handles a single command. Returns `true` if the loop should exit.
fn handle_cmd(
    cmd: StepCmd,
    out: &mut impl Write,
    current_msg: &mut Option<String>,
    frame_idx: &mut usize,
) -> IoResult<bool> {
    match cmd {
        StepCmd::Start(msg) => {
            if let Some(prev) = current_msg.take() {
                freeze_success(out, &prev)?;
            }
            *frame_idx = 0;
            render_frame(out, frame_at(0), &msg)?;
            *frame_idx = 1;
            *current_msg = Some(msg);
            Ok(false)
        }
        StepCmd::Complete(msg) => {
            *current_msg = None;
            freeze_success(out, &msg)?;
            Ok(false)
        }
        StepCmd::Fail(msg) => {
            *current_msg = None;
            freeze_fail(out, &msg)?;
            Ok(false)
        }
        StepCmd::Stop => {
            if let Some(prev) = current_msg.take() {
                freeze_success(out, &prev)?;
            }
            Ok(true)
        }
    }
}

/// Advances the spinner by one frame while waiting for a command.
fn advance_frame(
    out: &mut impl Write,
    current_msg: &mut Option<String>,
    frame_idx: &mut usize,
) -> IoResult<()> {
    if let Some(msg) = current_msg.as_ref() {
        let frame = frame_at(frame_idx.rem_euclid(FRAMES.len()));
        render_frame(out, frame, msg)?;
        *frame_idx = frame_idx.saturating_add(1);
    }
    Ok(())
}

/// The render loop for the steps component.
async fn render_loop(mut rx: mpsc::Receiver<StepCmd>) -> IoResult<()> {
    let mut out = stdout();
    let mut frame_idx: usize = 0;
    let mut current_msg: Option<String> = None;

    out.queue(cursor::Hide)?;
    out.flush()?;

    loop {
        let cmd_result = if current_msg.is_some() {
            timeout(FRAME_DURATION, rx.recv()).await
        } else {
            Ok(rx.recv().await)
        };

        let mut should_break = false;
        match cmd_result {
            Ok(Some(cmd)) => {
                should_break = handle_cmd(cmd, &mut out, &mut current_msg, &mut frame_idx)?;
            }
            Ok(None) => should_break = true,
            Err(_) => advance_frame(&mut out, &mut current_msg, &mut frame_idx)?,
        }
        if should_break {
            break;
        }
    }

    out.queue(cursor::Show)?;
    out.flush()?;
    Ok(())
}

/// Spawns the render loop and reports terminal write failures.
async fn render(rx: mpsc::Receiver<StepCmd>) {
    if let Err(err) = render_loop(rx).await {
        eprintln!("step render failed: {err}");
    }
}

/// Returns the spinner frame at the given index.
fn frame_at(idx: usize) -> &'static str {
    FRAMES.get(idx).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf() -> Vec<u8> {
        Vec::new()
    }

    fn output(buf: &[u8]) -> String {
        String::from_utf8_lossy(buf).into_owned()
    }

    #[test]
    fn freeze_success_writes_checkmark_and_message() {
        // ARRANGE
        let mut buffer = buf();

        // ACT
        freeze_success(&mut buffer, "done").unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(out.contains("done"), "output: {out}");
        assert!(out.contains('\u{2713}'), "output: {out}");
    }

    #[test]
    fn freeze_fail_writes_cross_and_message() {
        // ARRANGE
        let mut buffer = buf();

        // ACT
        freeze_fail(&mut buffer, "oops").unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(out.contains("oops"), "output: {out}");
        assert!(out.contains('\u{2717}'), "output: {out}");
    }

    #[test]
    fn render_frame_writes_frame_and_message() {
        // ARRANGE
        let mut buffer = buf();

        // ACT
        render_frame(&mut buffer, frame_at(0), "loading").unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(out.contains("loading"), "output: {out}");
        assert!(out.contains(frame_at(0)), "output: {out}");
    }

    #[test]
    fn handle_cmd_start_sets_current_msg() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = None;
        let mut idx = 0_usize;

        // ACT
        let done = handle_cmd(
            StepCmd::Start("step one".into()),
            &mut buffer,
            &mut msg,
            &mut idx,
        )
        .unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(!done);
        assert_eq!(msg.as_deref(), Some("step one"));
        assert_eq!(idx, 1);
        assert!(out.contains("step one"), "output: {out}");
    }

    #[test]
    fn handle_cmd_start_freezes_previous_msg() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = Some("old step".into());
        let mut idx = 0_usize;

        // ACT
        handle_cmd(
            StepCmd::Start("new step".into()),
            &mut buffer,
            &mut msg,
            &mut idx,
        )
        .unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(out.contains("old step"), "output: {out}");
        assert!(out.contains('\u{2713}'), "output: {out}");
        assert!(out.contains("new step"), "output: {out}");
    }

    #[test]
    fn handle_cmd_complete_freezes_as_success() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = Some("current".into());
        let mut idx = 0_usize;

        // ACT
        let done = handle_cmd(
            StepCmd::Complete("all done".into()),
            &mut buffer,
            &mut msg,
            &mut idx,
        )
        .unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(!done);
        assert!(msg.is_none());
        assert!(out.contains("all done"), "output: {out}");
        assert!(out.contains('\u{2713}'), "output: {out}");
    }

    #[test]
    fn handle_cmd_fail_freezes_as_failure() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = Some("current".into());
        let mut idx = 0_usize;

        // ACT
        let done = handle_cmd(
            StepCmd::Fail("broke".into()),
            &mut buffer,
            &mut msg,
            &mut idx,
        )
        .unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(!done);
        assert!(msg.is_none());
        assert!(out.contains("broke"), "output: {out}");
        assert!(out.contains('\u{2717}'), "output: {out}");
    }

    #[test]
    fn handle_cmd_stop_returns_true_and_freezes_current() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = Some("last step".into());
        let mut idx = 0_usize;

        // ACT
        let done = handle_cmd(StepCmd::Stop, &mut buffer, &mut msg, &mut idx).unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(done);
        assert!(msg.is_none());
        assert!(out.contains("last step"), "output: {out}");
        assert!(out.contains('\u{2713}'), "output: {out}");
    }

    #[test]
    fn handle_cmd_stop_with_no_current_msg_returns_true() {
        // ARRANGE
        let mut buffer = buf();
        let mut msg: Option<String> = None;
        let mut idx = 0_usize;

        // ACT
        let done = handle_cmd(StepCmd::Stop, &mut buffer, &mut msg, &mut idx).unwrap();
        let out = output(&buffer);

        // ASSERT
        assert!(done);
        assert!(!out.contains('\u{2713}'), "expected no checkmark: {out}");
    }
}
