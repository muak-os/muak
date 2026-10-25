//! Progress value formatting.

use super::{State, Unit};
use crate::glyphs::Glyphs;
use crate::style::{Role, Theme};
use crate::text::{self, Line, Span};
use crate::units;

pub(super) const RATE_MIN_WIDTH: usize = 80;
const BAR_MIN_WIDTH: usize = 41;
const MIN_LABEL_WIDTH: usize = 8;
const MAX_BAR_WIDTH: usize = 40;

pub(super) fn count_text(unit: Unit, current: u64, total: u64) -> String {
    match unit {
        Unit::Bytes => format!("{} / {}", units::bytes(current), units::bytes(total)),
        Unit::Items => format!("{current}/{total}"),
    }
}

pub(super) fn rate_text(state: &State<'_>) -> Option<String> {
    if state.unit != Unit::Bytes {
        return None;
    }

    state.rate.map(|rate| format!("{}/s", units::bytes(rate)))
}

pub(super) fn eta_text(state: &State<'_>) -> Option<String> {
    if state.unit != Unit::Bytes {
        return None;
    }

    state.eta.map(|eta| format!("ETA {}", super::clock(eta)))
}

pub(super) fn suffix_width(
    rate: Option<&str>,
    eta: Option<&str>,
    width: usize,
    unit: Unit,
) -> usize {
    if width < RATE_MIN_WIDTH || unit != Unit::Bytes {
        return 0;
    }

    rate.map_or(0, |text| text::width(text).saturating_add(1))
        .saturating_add(eta.map_or(0, |text| text::width(text).saturating_add(1)))
}

pub(super) fn bar_width(width: usize, fixed: usize) -> usize {
    if width < BAR_MIN_WIDTH {
        return 0;
    }

    let available = width.saturating_sub(fixed);
    let bar = available.saturating_sub(MIN_LABEL_WIDTH);

    if bar < 3 { 0 } else { bar.min(MAX_BAR_WIDTH) }
}

pub(super) fn compact<'a>(
    label: &'a str,
    count: &str,
    percent: &str,
    width: usize,
    theme: Theme,
) -> Line<'a> {
    let mut line = Line::new(0);
    if width <= 2 {
        return line;
    }

    let count_width = text::width(count);
    let percent_width = text::width(percent);
    let with_count = 2_usize
        .saturating_add(1)
        .saturating_add(count_width)
        .saturating_add(1)
        .saturating_add(percent_width);
    let label_max = if width > with_count {
        width.saturating_sub(with_count)
    } else {
        width
            .saturating_sub(2)
            .saturating_sub(1)
            .saturating_sub(percent_width)
    };
    let label = text::truncate(label, label_max);
    line.push(Span::plain("  "));
    line.push(Span::plain(label));
    if width > with_count {
        line.push(Span::plain(" "));
        line.push(Span::styled(count.to_owned(), theme.style(Role::Muted)));
    }
    if !label.is_empty() || width > with_count {
        line.push(Span::plain(" "));
    }
    line.push(Span::styled(
        percent.to_owned(),
        theme.style(Role::Highlight),
    ));

    line
}

pub(super) fn push_bar(
    line: &mut Line<'_>,
    current: u64,
    total: u64,
    bar_width: usize,
    theme: Theme,
    glyphs: Glyphs,
) {
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
        line.push(Span::styled(glyphs.bar_filled.repeat(filled), style));
    }
    if empty > 0 {
        line.push(Span::styled(glyphs.bar_empty.repeat(empty), style));
    }
}

pub(super) fn percent(current: u64, total: u64) -> u64 {
    current
        .min(total)
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    #[test]
    fn count_text_separates_units_from_values() {
        // ARRANGE
        let bytes = count_text(Unit::Bytes, 1024, 2048);
        let items = count_text(Unit::Items, 5, 7);

        // ACT / ASSERT
        assert_eq!(bytes, "1 KiB / 2 KiB");
        assert_eq!(items, "5/7");
    }

    #[test]
    fn percent_clamps_zero_totals() {
        // ARRANGE
        let zero = percent(0, 0);
        let full = percent(20, 10);

        // ACT / ASSERT
        assert_eq!(zero, 0);
        assert_eq!(full, 100);
    }

    #[test]
    fn bar_width_respects_the_minimum_column() {
        // ARRANGE
        let narrow = bar_width(40, 10);
        let wide = bar_width(120, 10);

        // ACT / ASSERT
        assert_eq!(narrow, 0);
        assert!(wide > 0);
    }

    #[test]
    fn suffix_is_hidden_below_eighty_columns() {
        // ARRANGE
        let hidden = suffix_width(Some("1 KiB/s"), Some("ETA 00:02"), 60, Unit::Bytes);
        let shown = suffix_width(Some("1 KiB/s"), Some("ETA 00:02"), 80, Unit::Bytes);

        // ACT / ASSERT
        assert_eq!(hidden, 0);
        assert!(shown > 0);
    }

    #[test]
    fn eta_is_hidden_for_item_counts() {
        // ARRANGE
        let state = State {
            label: "items",
            current: 5,
            total: 7,
            unit: Unit::Items,
            rate: Some(1024),
            eta: Some(Duration::from_secs(2)),
        };

        // ACT
        let rate = rate_text(&state);
        let eta = eta_text(&state);

        // ASSERT
        assert_eq!(rate, None);
        assert_eq!(eta, None);
    }
}
