//! Single-line progress rendering.

use crate::style::{Role, Theme};
use crate::text::{self, Line, Span};
use crate::units;

/// State of a progress line.
#[derive(Clone, Copy, Debug)]
pub struct State<'a> {
    /// Label shown before the bar.
    pub label: &'a str,
    /// Completed amount.
    pub current: u64,
    /// Total amount.
    pub total: u64,
}

/// Builds a progress line that fits in `width` columns.
#[must_use]
pub fn line<'a>(state: &State<'a>, width: usize, theme: Theme) -> Line<'a> {
    if width == 0 {
        return Line::new(0);
    }

    let current = state.current.min(state.total);
    let percent = percent(current, state.total);
    let percent_text = format!("{percent}%");
    let sizes = format!(
        "({} / {})",
        units::bytes(current),
        units::bytes(state.total)
    );
    let fixed = 5_usize
        .saturating_add(text::width(&percent_text))
        .saturating_add(text::width(&sizes));
    let label_max = width.saturating_sub(fixed).saturating_sub(1);

    if label_max == 0 {
        return compact(state.label, &percent_text, width, theme);
    }

    let label = text::truncate(state.label, label_max);
    let label_width = text::width(label);
    let bar_width = width
        .saturating_sub(fixed)
        .saturating_sub(label_width)
        .clamp(1, 40);
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::plain(label));
    line.push(Span::plain(" "));
    push_bar(&mut line, current, state.total, bar_width, theme);
    line.push(Span::plain(" "));
    line.push(Span::styled(percent_text, theme.style(Role::Highlight)));
    line.push(Span::plain(" "));
    line.push(Span::styled(sizes, theme.style(Role::Muted)));

    line
}

fn compact<'a>(label: &'a str, percent: &str, width: usize, theme: Theme) -> Line<'a> {
    let mut line = Line::new(0);
    if width <= 2 {
        return line;
    }

    let percent_width = text::width(percent);
    let label_max = width
        .saturating_sub(2)
        .saturating_sub(1)
        .saturating_sub(percent_width);
    let label = text::truncate(label, label_max);
    line.push(Span::plain("  "));
    line.push(Span::plain(label));
    if !label.is_empty() {
        line.push(Span::plain(" "));
    }
    line.push(Span::styled(
        percent.to_owned(),
        theme.style(Role::Highlight),
    ));

    line
}

fn push_bar(line: &mut Line<'_>, current: u64, total: u64, bar_width: usize, theme: Theme) {
    let filled = if total == 0 {
        0
    } else {
        let width = u64::try_from(bar_width).unwrap_or(u64::MAX);
        let filled = current
            .saturating_mul(width)
            .checked_div(total)
            .unwrap_or(0);
        usize::try_from(filled).unwrap_or(bar_width).min(bar_width)
    };
    let empty = bar_width.saturating_sub(filled);
    let style = theme.style(Role::Accent);

    if filled > 0 {
        line.push(Span::styled("\u{2588}".repeat(filled), style));
    }
    if empty > 0 {
        line.push(Span::styled("\u{2591}".repeat(empty), style));
    }
}

fn percent(current: u64, total: u64) -> u64 {
    current.saturating_mul(100).checked_div(total).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(line: &Line<'_>) -> String {
        line.spans().iter().map(Span::text).collect()
    }

    #[test]
    fn zero_total_renders_zero_percent() {
        // ARRANGE
        let state = State {
            label: "download",
            current: 0,
            total: 0,
        };

        // ACT
        let line = line(&state, 60, Theme::plain());

        // ASSERT
        assert!(rendered(&line).contains("0%"));
        assert!(line.width() <= 60);
    }

    #[test]
    fn current_above_total_is_clamped() {
        // ARRANGE
        let state = State {
            label: "download",
            current: 20,
            total: 10,
        };

        // ACT
        let line = line(&state, 60, Theme::plain());

        // ASSERT
        assert!(rendered(&line).contains("100%"));
    }

    #[test]
    fn line_fits_a_narrow_terminal() {
        // ARRANGE
        let state = State {
            label: "a very long label that must be truncated",
            current: 5,
            total: 10,
        };

        // ACT
        let line = line(&state, 24, Theme::plain());

        // ASSERT
        assert!(line.width() <= 24);
        assert!(rendered(&line).contains("50%"));
    }

    #[test]
    fn line_falls_back_to_a_compact_form() {
        // ARRANGE
        let state = State {
            label: "download",
            current: 1,
            total: 2,
        };

        // ACT
        let line = line(&state, 12, Theme::plain());

        // ASSERT
        assert!(line.width() <= 12);
        assert!(!rendered(&line).contains('\u{2588}'));
    }

    #[test]
    fn wide_labels_are_truncated_on_a_boundary() {
        // ARRANGE
        let state = State {
            label: "\u{65e5}\u{672c}\u{8a9e}\u{65e5}\u{672c}\u{8a9e}",
            current: 3,
            total: 4,
        };

        // ACT
        let line = line(&state, 30, Theme::plain());

        // ASSERT
        assert!(line.width() <= 30);
    }
}
