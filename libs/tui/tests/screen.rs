mod common;

#[cfg(test)]
mod tests {
    use tui::screen::{Ansi, Screen as _};
    use tui::size::Size;
    use tui::style::{self, AnsiColor};
    use tui::text::{Line, Span};

    use crate::common::strip_ansi;

    fn screen() -> Ansi<Vec<u8>> {
        Ansi::new(Vec::new(), Size::new(80, 25))
    }

    #[test]
    fn move_to_writes_cursor_position() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.move_to(2, 3).unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[4;3H".to_vec());
    }

    #[test]
    fn move_to_column_writes_column_position() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.move_to_column(2).unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[3G".to_vec());
    }

    #[test]
    fn clear_until_newline_clears_the_tail() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.clear_until_newline().unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[K".to_vec());
    }

    #[test]
    fn clear_line_parks_at_the_row_start() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.clear_line(2).unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[3;1H\x1b[K".to_vec());
    }

    #[test]
    fn clear_all_clears_the_surface() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.clear_all().unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[2J".to_vec());
    }

    #[test]
    fn scroll_region_round_trips() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.set_scroll_region(1, 9).unwrap();
        screen.reset_scroll_region().unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[2;10r\x1b[r".to_vec());
    }

    #[test]
    fn cursor_can_be_hidden_and_shown() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.hide_cursor().unwrap();
        screen.show_cursor().unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[?25l\x1b[?25h".to_vec());
    }

    #[test]
    fn write_inline_emits_styled_text() {
        // ARRANGE
        let mut screen = screen();
        let mut line = Line::new(0);
        line.push(Span::styled("hello", style::fg(AnsiColor::Red)));

        // ACT
        screen.write_inline(&line).unwrap();
        let output = screen.into_inner();

        // ASSERT
        assert!(output.starts_with(b"\x1b[31m"));
        assert!(output.ends_with(b"\x1b[0m"));
        assert_eq!(strip_ansi(&output), "hello");
    }

    #[test]
    fn write_line_honors_the_start_column() {
        // ARRANGE
        let mut screen = screen();
        let line = Line::plain(5, "hi");

        // ACT
        screen.write_line(2, &line).unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\x1b[3;6Hhi".to_vec());
    }

    #[test]
    fn newline_writes_a_single_line_feed() {
        // ARRANGE
        let mut screen = screen();

        // ACT
        screen.newline().unwrap();

        // ASSERT
        assert_eq!(screen.into_inner(), b"\n".to_vec());
    }

    #[test]
    fn ansi_exposes_the_surface_size() {
        // ARRANGE
        let size = Size::new(100, 40);

        // ACT
        let screen = Ansi::new(Vec::new(), size);

        // ASSERT
        assert_eq!(screen.size(), size);
    }
}
