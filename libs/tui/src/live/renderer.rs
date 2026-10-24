//! Renderer thread plumbing and shared line builders.

use alloc::borrow::Cow;
use core::time::Duration;
use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

use crate::screen::Screen;
use crate::style::{Role, Theme};
use crate::text::{self, Line, Span};

pub(crate) const FRAME_DURATION: Duration = Duration::from_millis(80);

const CHANNEL_CAPACITY: usize = 64;
const FRAMES: &[&str] = &[
    "\u{2807}", "\u{281b}", "\u{2819}", "\u{2838}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{280f}",
];

/// Cloneable sender for renderer commands.
pub(crate) struct Handle<C> {
    tx: SyncSender<C>,
}

/// Owns a renderer thread and its command channel.
pub(crate) struct Renderer<C>
where
    C: Send + Clone + 'static,
{
    handle: Option<Handle<C>>,
    join: Option<JoinHandle<io::Result<()>>>,
    stop: C,
}

impl<C> Clone for Handle<C> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<C> Handle<C> {
    /// Sends `command`, dropping it when the channel is full.
    pub(crate) fn send(&self, command: C) {
        let _send_result = self.tx.try_send(command);
    }

    /// Sends `command`, waiting for room when the channel is full.
    pub(crate) fn send_waiting(&self, command: C) {
        let _send_result = self.tx.send(command);
    }
}

impl<C> Renderer<C>
where
    C: Send + Clone + 'static,
{
    /// Starts a named renderer thread.
    pub(crate) fn start<F>(name: &str, stop: C, run: F) -> io::Result<Self>
    where
        F: FnOnce(&Receiver<C>) -> io::Result<()> + Send + 'static,
    {
        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let join = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run(&rx))?;

        Ok(Self {
            handle: Some(Handle { tx }),
            join: Some(join),
            stop,
        })
    }

    /// Returns a cloneable command handle while the thread is running.
    pub(crate) fn handle(&self) -> Option<Handle<C>> {
        self.handle.clone()
    }

    /// Stops the thread and waits for it to finish.
    pub(crate) fn finish(mut self) -> io::Result<()> {
        if let Some(handle) = self.handle.take() {
            handle.send_waiting(self.stop.clone());
        }

        self.join_thread()
    }

    fn stop_thread(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.send_waiting(self.stop.clone());
        }
    }

    fn join_thread(&mut self) -> io::Result<()> {
        self.join.take().map_or(Ok(()), |join| match join.join() {
            Ok(result) => result,
            Err(_panic) => Err(io::Error::other("renderer thread panicked")),
        })
    }
}

impl<C> Drop for Renderer<C>
where
    C: Send + Clone + 'static,
{
    fn drop(&mut self) {
        self.stop_thread();
        let _join_result = self.join_thread();
    }
}

/// Returns the spinner frame at `index`.
pub(crate) fn frame_at(index: usize) -> &'static str {
    let modulo = FRAMES.len();
    let safe = index.checked_rem(modulo).unwrap_or(0);

    FRAMES.get(safe).copied().unwrap_or("")
}

/// Builds an active spinner line that fits in `width` columns.
pub(crate) fn spinner_line(
    message: &str,
    frame: usize,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let frame_text = frame_at(frame);
    let prefix = 2_usize
        .saturating_add(text::width(frame_text))
        .saturating_add(1);
    let label = fitted(message, width.saturating_sub(prefix));
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(frame_text, theme.style(Role::Accent)));
    line.push(Span::plain(" "));
    line.push(Span::plain(label));

    line
}

/// Builds a plain message line that fits in `width` columns.
pub(crate) fn message_line(message: &str, width: usize) -> Line<'static> {
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::plain(fitted(message, width.saturating_sub(2))));

    line
}

/// Builds a frozen success or failure line that fits in `width` columns.
pub(crate) fn status_line(
    message: &str,
    success: bool,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let (mark, role) = if success {
        ("\u{2713}", Role::Success)
    } else {
        ("\u{2717}", Role::Error)
    };
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(mark, theme.style(role)));
    line.push(Span::plain(" "));
    line.push(Span::plain(fitted(message, width.saturating_sub(4))));

    line
}

/// Draws `line` on the active line and flushes it.
pub(crate) fn write_active<S: Screen>(screen: &mut S, line: &Line<'_>) -> io::Result<()> {
    screen.move_to_column(0)?;
    screen.clear_until_newline()?;
    screen.write_inline(line)?;

    screen.flush()
}

/// Draws `line` and moves to the next line.
pub(crate) fn write_frozen<S: Screen>(screen: &mut S, line: &Line<'_>) -> io::Result<()> {
    screen.move_to_column(0)?;
    screen.clear_until_newline()?;
    screen.write_inline(line)?;
    screen.newline()?;

    screen.flush()
}

/// Draws `line` without cursor control and moves to the next line.
pub(crate) fn write_plain<S: Screen>(screen: &mut S, line: &Line<'_>) -> io::Result<()> {
    screen.write_inline(line)?;
    screen.newline()?;

    screen.flush()
}

fn fitted(message: &str, width: usize) -> String {
    let sanitized = text::sanitize(Cow::Borrowed(message));

    text::truncate(&sanitized, width).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_waiting_waits_for_capacity() {
        // ARRANGE
        let (tx, rx) = sync_channel(1);
        let handle = Handle { tx };
        handle.send(1_u8);
        let sender = thread::spawn(move || handle.send_waiting(2_u8));

        // ACT
        let first = rx.recv_timeout(Duration::from_secs(1));
        sender.join().unwrap();
        let second = rx.recv_timeout(Duration::from_secs(1));

        // ASSERT
        assert_eq!(first, Ok(1));
        assert_eq!(second, Ok(2));
    }
}
