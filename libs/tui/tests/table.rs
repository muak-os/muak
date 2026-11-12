pub mod common;

#[cfg(test)]
mod tests {
    use tui::screen::ansi::Ansi;
    use tui::size::Size;
    use tui::style::{self, AnsiColor, Theme};
    use tui::table::{Cell, Table};
    use tui::text::{self, Span};

    use crate::common::strip_ansi;

    fn render(table: &Table, theme: Theme) -> String {
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 25));
        table.render(&mut screen, theme).unwrap();

        strip_ansi(&screen.into_inner())
    }

    fn render_bytes(table: &Table, theme: Theme) -> Vec<u8> {
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 25));
        table.render(&mut screen, theme).unwrap();

        screen.into_inner()
    }

    fn render_fitted(table: &Table, theme: Theme, cols: u16) -> String {
        let mut screen = Ansi::new(Vec::new(), Size::new(cols, 25));
        table.render_fitted(&mut screen, theme, cols).unwrap();

        strip_ansi(&screen.into_inner())
    }

    fn render_fitted_bytes(table: &Table, theme: Theme, cols: u16) -> Vec<u8> {
        let mut screen = Ansi::new(Vec::new(), Size::new(cols, 25));
        table.render_fitted(&mut screen, theme, cols).unwrap();

        screen.into_inner()
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
    fn fitted_empty_table_renders_nothing() {
        // ARRANGE
        let table = Table::new();

        // ACT
        let output = render_fitted(&table, Theme::plain(), 20);

        // ASSERT
        assert_eq!(output, "");
    }

    #[test]
    fn fitted_unfittable_table_stays_lossless() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["a", "running"]);

        // ACT
        let natural = render_bytes(&table, Theme::plain());
        let fitted = render_fitted_bytes(&table, Theme::plain(), 10);

        // ASSERT
        assert_eq!(fitted, natural);
    }

    #[test]
    fn fitted_single_column_table_stays_lossless() {
        // ARRANGE
        let table = Table::new().header(&["NAME"]).row(&["a-very-long-name"]);

        // ACT
        let natural = render_bytes(&table, Theme::plain());
        let fitted = render_fitted_bytes(&table, Theme::plain(), 8);

        // ASSERT
        assert_eq!(fitted, natural);
    }

    #[test]
    fn fitted_table_with_broken_terminal_width_renders_naturally() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["a", "running"]);

        // ACT
        let natural = render_bytes(&table, Theme::plain());
        let none = render_fitted_bytes(&table, Theme::plain(), 0);
        let one = render_fitted_bytes(&table, Theme::plain(), 1);

        // ASSERT
        assert_eq!(none, natural);
        assert_eq!(one, natural);
    }

    #[test]
    fn fitted_headerless_column_keeps_a_readable_floor() {
        // ARRANGE
        let table = Table::new().header(&["A"]).row(&["x", "extra-columns"]);

        // ACT
        let output = render_fitted(&table, Theme::plain(), 8);

        // ASSERT
        assert_eq!(output, "A\nx  ex\u{2026}ns\n");
    }

    #[test]
    fn fitted_wide_cells_fill_the_budget_and_keep_alignment() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATE"])
            .row(&["\u{65e5}\u{672c}\u{8a9e}", "running"]);

        // ACT
        let output = render_fitted(&table, Theme::plain(), 13);

        // ASSERT
        assert_eq!(
            output,
            "NAME    STATE\n\u{65e5}\u{672c}\u{8a9e}  ru\u{2026}ng\n"
        );
        for line in output.lines() {
            assert!(
                text::width(line) <= 13,
                "line {line:?} overflows the budget"
            );
        }
    }

    #[test]
    fn fitted_multi_column_rows_align_after_elision() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "WWID", "STATE"])
            .row(&["disk", "abcdefghijklmnop", "ok"])
            .row(&["nvme", "qrstuvwxyzABCDEF", "ok"]);

        // ACT
        let output = render_fitted(&table, Theme::plain(), 20);

        // ASSERT
        assert_eq!(
            output,
            "NAME  WWID     STATE\ndisk  abc\u{2026}nop  ok\nnvme  qrs\u{2026}DEF  ok\n"
        );
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
    fn fitted_table_keeps_lines_within_the_budget() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "WWID"])
            .row(&["disk", "0123456789abcdef0123456789abcdef"]);
        let budget: u16 = 24;

        // ACT
        let output = render_fitted(&table, Theme::plain(), budget);

        // ASSERT
        assert_ne!(output, "");
        for line in output.lines() {
            assert!(
                text::width(line) <= usize::from(budget),
                "line {line:?} overflows the budget"
            );
        }
    }

    #[test]
    fn fitted_table_middle_elides_a_shrunk_cell() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "WWID"])
            .row(&["disk", "abcdefghijklmnopqrstuvwxyz"]);

        // ACT
        let output = render_fitted(&table, Theme::plain(), 20);

        // ASSERT
        assert!(output.contains("abcdef\u{2026}tuvwxyz"));
        assert!(!output.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn fitted_table_that_fits_matches_the_natural_render() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "STATUS"])
            .row(&["disk", "ok"])
            .sub_row("\u{2514}", &["child", "ok"]);

        // ACT
        let natural = render_bytes(&table, Theme::plain());
        let fitted = render_fitted_bytes(&table, Theme::plain(), 80);

        // ASSERT
        assert_eq!(fitted, natural);
    }

    #[test]
    fn unfitted_budget_beyond_eighty_columns_keeps_wide_values_whole() {
        // ARRANGE
        let wwid =
            "naa.6001405d3f2b1c4a8e9f0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
        let table = Table::new().header(&["NAME", "WWID"]).row(&["disk", wwid]);

        // ACT
        let natural = render_bytes(&table, Theme::plain());
        let fitted = render_fitted_bytes(&table, Theme::plain(), 80);
        let unfitted = render_fitted_bytes(&table, Theme::plain(), u16::MAX);

        // ASSERT
        assert!(natural.len() > 80);
        assert!(strip_ansi(&natural).contains(wwid));
        assert_ne!(fitted, natural);
        assert_eq!(unfitted, natural);
    }

    #[test]
    fn fitted_sub_row_keeps_the_tree_prefix() {
        // ARRANGE
        let table = Table::new()
            .header(&["NAME", "S"])
            .row(&["parent", "running"])
            .sub_row("\u{2514}", &["child", "running"]);

        // ACT
        let output = render_fitted(&table, Theme::plain(), 16);

        // ASSERT
        assert!(output.contains("  \u{2514} child"));
        assert!(output.contains('\u{2026}'));
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
