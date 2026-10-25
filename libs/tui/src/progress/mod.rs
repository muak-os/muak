//! Single-line progress rendering.

mod format;

use core::time::Duration;

use self::format::{
    RATE_MIN_WIDTH, bar_width, compact, count_text, eta_text, percent, push_bar, rate_text,
    suffix_width,
};
use crate::glyphs::Glyphs;
use crate::style::{Role, Theme};
use crate::text::{self, Line, Span};

/// Unit of a progress value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Byte counts rendered with IEC units.
    Bytes,
    /// Item counts rendered as `current/total`.
    Items,
}

/// State of a progress line.
#[derive(Clone, Copy, Debug)]
pub struct State<'a> {
    /// Label shown before the bar.
    pub label: &'a str,
    /// Completed amount.
    pub current: u64,
    /// Total amount.
    pub total: u64,
    /// Unit of the amounts.
    pub unit: Unit,
    /// Smoothed transfer rate in bytes per second.
    pub rate: Option<u64>,
    /// Estimated time remaining.
    pub eta: Option<Duration>,
}

/// Builds a progress line that fits in `width` columns.
#[must_use]
pub fn line<'a>(state: &State<'a>, width: usize, theme: Theme, glyphs: Glyphs) -> Line<'a> {
    if width == 0 {
        return Line::new(0);
    }

    let current = state.current.min(state.total);
    let percent_text = format!("{}%", percent(current, state.total));
    let count = count_text(state.unit, current, state.total);
    let rate = rate_text(state);
    let eta = eta_text(state);
    let count_width = text::width(&count);
    let percent_width = text::width(&percent_text);
    let suffix = suffix_width(rate.as_deref(), eta.as_deref(), width, state.unit);
    let fixed = 2_usize
        .saturating_add(1)
        .saturating_add(count_width)
        .saturating_add(1)
        .saturating_add(percent_width)
        .saturating_add(suffix);
    let bar_width = bar_width(width, fixed);
    let bar_segment = if bar_width > 0 {
        bar_width.saturating_add(1)
    } else {
        0
    };
    let label_max = width.saturating_sub(fixed).saturating_sub(bar_segment);

    if label_max == 0 {
        return compact(state.label, &count, &percent_text, width, theme);
    }

    let label = text::truncate(state.label, label_max);
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::plain(label));
    line.push(Span::plain(" "));
    if bar_width > 0 {
        push_bar(&mut line, current, state.total, bar_width, theme, glyphs);
        line.push(Span::plain(" "));
    }
    line.push(Span::styled(count, theme.style(Role::Muted)));
    line.push(Span::plain(" "));
    line.push(Span::styled(percent_text, theme.style(Role::Highlight)));
    if let Some(rate) = rate.filter(|_| width >= RATE_MIN_WIDTH) {
        line.push(Span::plain(" "));
        line.push(Span::styled(rate, theme.style(Role::Muted)));
    }
    if let Some(eta) = eta.filter(|_| width >= RATE_MIN_WIDTH) {
        line.push(Span::plain(" "));
        line.push(Span::styled(eta, theme.style(Role::Muted)));
    }

    line
}

/// Formats `duration` as a compact clock value.
#[must_use]
pub fn clock(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total.checked_div(3600).unwrap_or(0);
    let minutes = total
        .checked_rem(3600)
        .unwrap_or(0)
        .checked_div(60)
        .unwrap_or(0);
    let seconds = total.checked_rem(60).unwrap_or(0);

    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(label: &str, current: u64, total: u64, unit: Unit) -> State<'_> {
        State {
            label,
            current,
            total,
            unit,
            rate: None,
            eta: None,
        }
    }

    fn rendered(line: &Line<'_>) -> String {
        line.spans().iter().map(Span::text).collect()
    }

    #[test]
    fn zero_total_renders_zero_percent() {
        // ARRANGE
        let state = state("download", 0, 0, Unit::Items);

        // ACT
        let line = line(&state, 60, Theme::plain(), Glyphs::plain());

        // ASSERT
        assert!(rendered(&line).contains("0%"));
        assert!(line.width() <= 60);
    }

    #[test]
    fn current_above_total_is_clamped() {
        // ARRANGE
        let state = state("download", 20, 10, Unit::Items);

        // ACT
        let line = line(&state, 60, Theme::plain(), Glyphs::plain());

        // ASSERT
        assert!(rendered(&line).contains("100%"));
    }

    #[test]
    fn item_counts_are_not_rendered_as_bytes() {
        // ARRANGE
        let state = state("download", 5, 7, Unit::Items);

        // ACT
        let line = line(&state, 80, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(text.contains("5/7"), "rendered: {text}");
        assert!(!text.contains("5 B"), "rendered: {text}");
    }

    #[test]
    fn bytes_use_iec_units() {
        // ARRANGE
        let mut state = state("download", 1024, 2048, Unit::Bytes);
        state.rate = Some(1024);

        // ACT
        let line = line(&state, 120, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(text.contains("1 KiB / 2 KiB"), "rendered: {text}");
        assert!(text.contains("1 KiB/s"), "rendered: {text}");
    }

    #[test]
    fn line_fits_a_narrow_terminal() {
        // ARRANGE
        let state = state(
            "a very long label that must be truncated",
            5,
            10,
            Unit::Items,
        );

        // ACT
        let line = line(&state, 24, Theme::plain(), Glyphs::plain());

        // ASSERT
        assert!(line.width() <= 24);
        assert!(rendered(&line).contains("50%"));
    }

    #[test]
    fn line_falls_back_to_a_compact_form() {
        // ARRANGE
        let state = state("download", 1, 2, Unit::Items);

        // ACT
        let line = line(&state, 12, Theme::plain(), Glyphs::plain());

        // ASSERT
        assert!(line.width() <= 12);
        assert!(!rendered(&line).contains('#'));
    }

    #[test]
    fn forty_columns_hide_the_bar_and_keep_the_count() {
        // ARRANGE
        let state = state("download", 5, 7, Unit::Items);

        // ACT
        let line = line(&state, 40, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(line.width() <= 40);
        assert!(text.contains("5/7"), "rendered: {text}");
        assert!(!text.contains('#'), "rendered: {text}");
    }

    #[test]
    fn sixty_columns_show_the_bar_without_rate_or_eta() {
        // ARRANGE
        let mut state = state("download", 5, 7, Unit::Bytes);
        state.rate = Some(1024);
        state.eta = Some(Duration::from_secs(2));

        // ACT
        let line = line(&state, 60, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(line.width() <= 60);
        assert!(text.contains('#'), "rendered: {text}");
        assert!(!text.contains("/s"), "rendered: {text}");
        assert!(!text.contains("ETA"), "rendered: {text}");
    }

    #[test]
    fn eighty_columns_show_rate_and_eta() {
        // ARRANGE
        let mut state = state("download", 5, 7, Unit::Bytes);
        state.rate = Some(1024);
        state.eta = Some(Duration::from_secs(2));

        // ACT
        let line = line(&state, 80, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(line.width() <= 80);
        assert!(text.contains("1 KiB/s"), "rendered: {text}");
        assert!(text.contains("ETA 00:02"), "rendered: {text}");
    }

    #[test]
    fn one_hundred_twenty_columns_show_the_full_layout() {
        // ARRANGE
        let mut state = state("download", 5, 7, Unit::Bytes);
        state.rate = Some(1024);
        state.eta = Some(Duration::from_secs(2));

        // ACT
        let line = line(&state, 120, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(line.width() <= 120);
        assert!(text.contains('#'), "rendered: {text}");
        assert!(text.contains("1 KiB/s"), "rendered: {text}");
        assert!(text.contains("ETA 00:02"), "rendered: {text}");
    }

    #[test]
    fn every_supported_width_fits_its_line() {
        // ARRANGE
        let state = state("download", 5, 7, Unit::Items);

        // ACT / ASSERT
        for width in [40, 60, 80, 120] {
            let line = line(&state, width, Theme::plain(), Glyphs::plain());
            assert!(line.width() <= width, "width {width}: {}", rendered(&line));
        }
    }

    #[test]
    fn item_counts_never_show_rate_or_eta() {
        // ARRANGE
        let mut state = state("download", 5, 7, Unit::Items);
        state.rate = Some(1024);
        state.eta = Some(Duration::from_secs(2));

        // ACT
        let line = line(&state, 120, Theme::plain(), Glyphs::plain());
        let text = rendered(&line);

        // ASSERT
        assert!(!text.contains("/s"), "rendered: {text}");
        assert!(!text.contains("ETA"), "rendered: {text}");
    }

    #[test]
    fn wide_labels_are_truncated_on_a_boundary() {
        // ARRANGE
        let state = state(
            "\u{65e5}\u{672c}\u{8a9e}\u{65e5}\u{672c}\u{8a9e}",
            3,
            4,
            Unit::Items,
        );

        // ACT
        let line = line(&state, 30, Theme::plain(), Glyphs::plain());

        // ASSERT
        assert!(line.width() <= 30);
    }

    #[test]
    fn clock_formats_minutes_and_hours() {
        // ARRANGE
        let short = Duration::from_secs(62);
        let long = Duration::from_secs(3723);

        // ACT
        let short_text = clock(short);
        let long_text = clock(long);

        // ASSERT
        assert_eq!(short_text, "01:02");
        assert_eq!(long_text, "1:02:03");
    }
}
