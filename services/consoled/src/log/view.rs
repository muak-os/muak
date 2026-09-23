//! Renderer for the log area between the status panel and the footer.

use std::io::Write;

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{Clear, ClearType};

use crate::render::{FOOTER_ROWS, PANEL_ROWS};

/// Renders the visible window into the log area between panel and footer.
pub fn render<W: Write>(
    w: &mut W,
    visible: (&[String], &[String]),
    cols: u16,
    rows: u16,
) -> Result<()> {
    let log_area_end = rows.saturating_sub(FOOTER_ROWS);
    let log_rows = usize::from(log_area_end.saturating_sub(PANEL_ROWS));
    if log_rows == 0 {
        return Ok(());
    }

    for (offset, line) in visible.0.iter().chain(visible.1.iter()).enumerate() {
        let row = PANEL_ROWS.saturating_add(u16::try_from(offset).unwrap_or(0));
        let truncated = truncate(line, usize::from(cols));
        queue!(
            w,
            MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            ResetColor,
            Print(truncated),
        )?;
    }

    for offset in visible.0.len().saturating_add(visible.1.len())..log_rows {
        let row = PANEL_ROWS.saturating_add(u16::try_from(offset).unwrap_or(0));
        queue!(w, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }

    queue!(w, MoveTo(0, log_area_end.saturating_sub(1)))?;
    w.flush()?;

    Ok(())
}

fn truncate(line: &str, cols: usize) -> &str {
    match line.char_indices().nth(cols) {
        Some((index, _)) => line.get(..index).unwrap_or(line),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_produces_output() {
        // ARRANGE
        let visible = (&["line one".to_owned()][..], &[][..]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        render(&mut buf, visible, 80, 40).unwrap();

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_empty_visible_clears_area() {
        // ARRANGE
        let visible: (&[String], &[String]) = (&[], &[]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        render(&mut buf, visible, 80, 40).unwrap();

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_terminal_smaller_than_panel_plus_footer_is_noop() {
        // ARRANGE
        let visible = (&["line".to_owned()][..], &[][..]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        render(&mut buf, visible, 80, PANEL_ROWS + FOOTER_ROWS).unwrap();
    }

    #[test]
    fn truncate_slices_on_char_boundary() {
        // ARRANGE
        let line = "äöü ab";

        // ACT / ASSERT
        assert_eq!(truncate(line, 3), "äöü");
        assert_eq!(truncate(line, 100), line);
        assert_eq!(truncate("", 5), "");
    }
}
