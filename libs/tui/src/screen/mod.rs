//! Terminal output port and ANSI backend.

use std::io;

mod ansi;

use crate::size::Size;
use crate::text::{Line, Span};

/// ANSI terminal output surface.
pub type Ansi<W> = ansi::Ansi<W>;

/// Terminal output surface.
pub trait Screen {
    /// Returns the surface size.
    fn size(&self) -> Size;

    /// Moves the cursor to `col`, `row`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn move_to(&mut self, col: u16, row: u16) -> io::Result<()>;

    /// Moves the cursor to `col` on the current row.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn move_to_column(&mut self, col: u16) -> io::Result<()>;

    /// Clears from the cursor to the end of the current line.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn clear_until_newline(&mut self) -> io::Result<()>;

    /// Writes a styled span at the current cursor.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn write_span(&mut self, span: &Span<'_>) -> io::Result<()>;

    /// Sets the scrolling region.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn set_scroll_region(&mut self, top: u16, bottom: u16) -> io::Result<()>;

    /// Resets the scrolling region.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn reset_scroll_region(&mut self) -> io::Result<()>;

    /// Hides the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn hide_cursor(&mut self) -> io::Result<()>;

    /// Shows the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn show_cursor(&mut self) -> io::Result<()>;

    /// Flushes pending output.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn flush(&mut self) -> io::Result<()>;

    /// Clears `row` and parks the cursor at its start.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn clear_line(&mut self, row: u16) -> io::Result<()> {
        self.move_to(0, row)?;
        self.clear_until_newline()
    }

    /// Clears every row.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn clear_all(&mut self) -> io::Result<()> {
        for row in 0..self.size().rows {
            self.clear_line(row)?;
        }

        Ok(())
    }

    /// Writes `line` at `row`, honoring `line.col()`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn write_line(&mut self, row: u16, line: &Line<'_>) -> io::Result<()> {
        self.move_to(line.col(), row)?;
        self.write_inline(line)
    }

    /// Writes `line` at the current cursor.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn write_inline(&mut self, line: &Line<'_>) -> io::Result<()> {
        for span in line.spans() {
            self.write_span(span)?;
        }

        Ok(())
    }

    /// Moves to the next line and returns to column zero.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying writer fails.
    fn newline(&mut self) -> io::Result<()> {
        self.write_span(&Span::plain("\n"))
    }
}
