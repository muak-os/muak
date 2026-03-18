//! Renderer for the log area between the status panel and the footer.

use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{Clear, ClearType};

use crate::render::{FOOTER_ROWS, PANEL_ROWS};

/// Renders pre-sliced visible lines into the log area between panel and footer.
pub fn render(w: &mut impl Write, visible: &[String], cols: u16, rows: u16) -> io::Result<()> {
    let log_area_end = rows.saturating_sub(FOOTER_ROWS);
    let log_rows = log_area_end.saturating_sub(PANEL_ROWS) as usize;
    if log_rows == 0 {
        return Ok(());
    }

    for (offset, line) in visible.iter().enumerate() {
        let row = PANEL_ROWS + offset as u16;
        let truncated: String = line.chars().take(cols as usize).collect();
        queue!(
            w,
            MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            ResetColor,
            Print(&truncated),
        )?;
    }

    for offset in visible.len()..log_rows {
        let row = PANEL_ROWS + offset as u16;
        queue!(w, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }

    queue!(w, MoveTo(0, log_area_end - 1))?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_produces_output() {
        // ARRANGE
        let visible = vec![
            "line one".to_owned(),
            "line two".to_owned(),
            "line three".to_owned(),
        ];
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render(&mut buf, &visible, 80, 40);

        // ASSERT
        assert!(result.is_ok());
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_empty_visible_clears_area() {
        // ARRANGE
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render(&mut buf, &[], 80, 40);

        // ASSERT
        assert!(result.is_ok());
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_terminal_smaller_than_panel_plus_footer_is_noop() {
        // ARRANGE
        let visible = vec!["line".to_owned()];
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render(&mut buf, &visible, 80, PANEL_ROWS + FOOTER_ROWS);

        // ASSERT
        assert!(result.is_ok());
    }
}
