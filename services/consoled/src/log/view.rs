//! Renderer for the log area between the status panel and the footer.

extern crate alloc;

use alloc::borrow::Cow;
use std::io;

use tui::screen::Screen;
use tui::text::{self, Line, Span};

use crate::render::{FOOTER_ROWS, PANEL_ROWS};

/// Renders the visible window into the log area between panel and footer.
pub fn render<S: Screen>(
    screen: &mut S,
    visible: (&[String], &[String]),
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let log_area_end = rows.saturating_sub(FOOTER_ROWS);
    let log_rows = usize::from(log_area_end.saturating_sub(PANEL_ROWS));
    if log_rows == 0 {
        return Ok(());
    }

    let width = usize::from(cols);

    for (offset, line) in visible.0.iter().chain(visible.1.iter()).enumerate() {
        let row = PANEL_ROWS.saturating_add(u16::try_from(offset).unwrap_or(0));
        let sanitized = text::sanitize(Cow::Borrowed(line));
        let truncated = text::truncate(&sanitized, width);
        let mut rendered = Line::new(0);
        rendered.push(Span::plain(truncated));
        screen.clear_line(row)?;
        screen.write_line(row, &rendered)?;
    }

    for offset in visible.0.len().saturating_add(visible.1.len())..log_rows {
        let row = PANEL_ROWS.saturating_add(u16::try_from(offset).unwrap_or(0));
        screen.clear_line(row)?;
    }

    screen.move_to(0, log_area_end.saturating_sub(1))?;

    screen.flush()
}

#[cfg(test)]
mod tests {
    use tui::screen::Ansi;
    use tui::size::Size;

    use super::*;

    fn screen() -> Ansi<Vec<u8>> {
        Ansi::new(Vec::new(), Size::new(80, 40))
    }

    #[test]
    fn render_produces_output() {
        // ARRANGE
        let visible = (&["line one".to_owned()][..], &[][..]);
        let mut screen = screen();

        // ACT
        render(&mut screen, visible, 80, 40).unwrap();

        // ASSERT
        assert_ne!(screen.into_inner(), b"");
    }

    #[test]
    fn render_empty_visible_clears_area() {
        // ARRANGE
        let visible: (&[String], &[String]) = (&[], &[]);
        let mut screen = screen();

        // ACT
        render(&mut screen, visible, 80, 40).unwrap();

        // ASSERT
        assert_ne!(screen.into_inner(), b"");
    }

    #[test]
    fn render_terminal_smaller_than_panel_plus_footer_is_noop() {
        // ARRANGE
        let visible = (&["line".to_owned()][..], &[][..]);
        let mut screen = screen();

        // ACT
        render(&mut screen, visible, 80, PANEL_ROWS + FOOTER_ROWS).unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"".to_vec());
    }

    #[test]
    fn render_sanitizes_tabs_before_truncating() {
        // ARRANGE
        let visible = (&["a\tb".to_owned()][..], &[][..]);
        let mut screen = screen();

        // ACT
        render(&mut screen, visible, 80, 40).unwrap();

        // ASSERT
        let bytes = screen.into_inner();
        let output = String::from_utf8_lossy(&bytes);
        assert!(output.contains("a    b"), "output: {output}");
    }
}
