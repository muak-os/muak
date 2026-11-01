//! Recording screen shared by live widget integration tests.

use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use tui::screen::Screen;
use tui::size::Size;
use tui::text::Span;

/// Records every written span through a channel.
pub struct Recorder {
    tx: SyncSender<String>,
    size: Size,
}

impl Recorder {
    /// Creates a recording screen and the channel receiving its output.
    #[must_use]
    pub fn new(size: Size) -> (Self, Receiver<String>) {
        let (tx, rx) = sync_channel(1024);

        (Self { tx, size }, rx)
    }

    fn send(&self, text: impl Into<String>) {
        let _send_result = self.tx.try_send(text.into());
    }
}

impl Screen for Recorder {
    fn size(&self) -> Size {
        self.size
    }

    fn move_to(&mut self, _col: u16, _row: u16) -> io::Result<()> {
        Ok(())
    }

    fn move_to_column(&mut self, _col: u16) -> io::Result<()> {
        Ok(())
    }

    fn clear_until_newline(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_span(&mut self, span: &Span<'_>) -> io::Result<()> {
        self.send(span.text());

        Ok(())
    }

    fn set_scroll_region(&mut self, _top: u16, _bottom: u16) -> io::Result<()> {
        Ok(())
    }

    fn reset_scroll_region(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Collects every recorded span until the recording screen is dropped.
#[must_use]
pub fn collect(rx: &Receiver<String>) -> String {
    rx.iter().collect()
}
