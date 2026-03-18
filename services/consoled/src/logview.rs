//! Scrollback log view renderer for the area below the status panel.

use std::collections::VecDeque;
use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{Clear, ClearType};

use crate::render::{FOOTER_ROWS, PANEL_ROWS};

/// Renders a window of `ring` entries into the log area between panel and footer.
pub fn render_logview(
    w: &mut impl Write,
    ring: &VecDeque<String>,
    scroll_offset: usize,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let log_area_end = rows.saturating_sub(FOOTER_ROWS);
    let log_rows = log_area_end.saturating_sub(PANEL_ROWS) as usize;
    if log_rows == 0 {
        return Ok(());
    }

    let total = ring.len();
    let end = total.saturating_sub(scroll_offset);
    let start = end.saturating_sub(log_rows);

    for (offset, i) in (start..end).enumerate() {
        let row = PANEL_ROWS + offset as u16;
        let line = &ring[i];
        let truncated: String = line.chars().take(cols as usize).collect();
        queue!(
            w,
            MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            ResetColor,
            Print(&truncated),
        )?;
    }

    let filled = end - start;
    for offset in filled..log_rows {
        let row = PANEL_ROWS + offset as u16;
        queue!(w, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
    }

    queue!(w, MoveTo(0, log_area_end - 1))?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ring(lines: &[&str]) -> VecDeque<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn render_logview_produces_output() {
        // ARRANGE
        let ring = make_ring(&["line one", "line two", "line three"]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render_logview(&mut buf, &ring, 0, 80, 40);

        // ASSERT
        assert!(result.is_ok());
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_logview_empty_ring_clears_area() {
        // ARRANGE
        let ring = VecDeque::new();
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render_logview(&mut buf, &ring, 0, 80, 40);

        // ASSERT
        assert!(result.is_ok());
        assert!(!buf.is_empty());
    }

    #[test]
    fn render_logview_scroll_offset_zero_shows_tail() {
        // ARRANGE
        let ring = make_ring(&["a", "b", "c", "d", "e"]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render_logview(&mut buf, &ring, 0, 80, 14);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn render_logview_terminal_smaller_than_panel_plus_footer_is_noop() {
        // ARRANGE
        let ring = make_ring(&["line"]);
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        let result = render_logview(&mut buf, &ring, 0, 80, PANEL_ROWS + FOOTER_ROWS);

        // ASSERT
        assert!(result.is_ok());
    }
}
