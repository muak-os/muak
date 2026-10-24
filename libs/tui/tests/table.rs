mod common;

#[cfg(test)]
mod tests {
    use tui::screen::Ansi;
    use tui::size::Size;
    use tui::style::{self, AnsiColor, Theme};
    use tui::table::{Cell, Table};
    use tui::text::Span;

    use crate::common::strip_ansi;

    fn render(table: &Table, theme: Theme) -> String {
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 25));
        table.render(&mut screen, theme).unwrap();

        strip_ansi(&screen.into_inner())
    }

    #[test]
    fn header_and_row_align_by_display_width() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["a", "running"]);

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "NAME  STATUS\na     running\n");
    }

    #[test]
    fn styled_cells_keep_alignment() {
        // ARRANGE
        let table = Table::new().header(&["STATE", "NOTE"]).row_cells(vec![
            Cell::styled("failed", style::fg(AnsiColor::Red)),
            Cell::plain("ok"),
        ]);

        // ACT
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 25));
        table.render(&mut screen, Theme::colored()).unwrap();
        let bytes = screen.into_inner();

        // ASSERT
        assert!(bytes.windows(5).any(|window| window == b"\x1b[31m"));
        assert_eq!(strip_ansi(&bytes), "STATE   NOTE\nfailed  ok\n");
    }

    #[test]
    fn wide_characters_keep_alignment() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["日本語", "ok"]);

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "NAME    STATUS\n日本語  ok\n");
    }

    #[test]
    fn sub_row_prefix_expands_the_first_column() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["parent", "ok"])
            .sub_row("└", &["child", "ok"]);

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "NAME       STATUS\nparent     ok\n  └ child  ok\n");
    }

    #[test]
    fn empty_table_renders_nothing() {
        // ARRANGE
        let table = Table::new();

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "");
    }

    #[test]
    fn uneven_rows_render_available_cells() {
        // ARRANGE
        let table = Table::new().header(&["A", "B"]).row(&["only"]);

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "A     B\nonly\n");
    }

    #[test]
    fn multi_span_cells_use_the_combined_width() {
        // ARRANGE
        let table = Table::new().header(&["NAME", "STATUS"]).row_cells(vec![
            Cell::from_spans(vec![Span::plain("ab"), Span::plain("cd")]),
            Cell::plain("ok"),
        ]);

        // ACT
        let output = render(&table, Theme::plain());

        // ASSERT
        assert_eq!(output, "NAME  STATUS\nabcd  ok\n");
    }
}
