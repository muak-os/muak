//! Renderer thread plumbing.

use core::time::Duration;
use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};

use crate::screen::Screen;
use crate::text::Line;

pub(crate) const FRAME_DURATION: Duration = Duration::from_millis(80);

const CHANNEL_CAPACITY: usize = 64;

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
