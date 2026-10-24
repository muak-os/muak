//! ANSI terminal backend.

use std::io::{self, Write};

use crate::screen::Screen;
use crate::size::Size;
use crate::text::{Span, write_span};

/// ANSI terminal output surface.
pub struct Ansi<W> {
    writer: W,
    size: Size,
}

impl<W> Ansi<W> {
    /// Wraps `writer` with a known surface size.
    #[must_use]
    pub const fn new(writer: W, size: Size) -> Self {
        Self { writer, size }
    }

    /// Returns the wrapped writer.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> Screen for Ansi<W> {
    fn size(&self) -> Size {
        self.size
    }

    fn move_to(&mut self, col: u16, row: u16) -> io::Result<()> {
        write!(
            self.writer,
            "\u{1b}[{};{}H",
            row.saturating_add(1),
            col.saturating_add(1)
        )
    }

    fn move_to_column(&mut self, col: u16) -> io::Result<()> {
        write!(self.writer, "\u{1b}[{}G", col.saturating_add(1))
    }

    fn clear_until_newline(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\x1b[K")
    }

    fn write_span(&mut self, span: &Span<'_>) -> io::Result<()> {
        write_span(&mut self.writer, span)
    }

    fn set_scroll_region(&mut self, top: u16, bottom: u16) -> io::Result<()> {
        write!(
            self.writer,
            "\u{1b}[{};{}r",
            top.saturating_add(1),
            bottom.saturating_add(1)
        )
    }

    fn reset_scroll_region(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\x1b[r")
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\x1b[?25l")
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\x1b[?25h")
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn clear_all(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\x1b[2J")
    }
}
