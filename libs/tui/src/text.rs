//! Styled text primitives.

use alloc::borrow::Cow;
use std::io::{self, Write};

use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::style::Style;

/// A single styled segment of text.
#[derive(Clone, Debug)]
pub struct Span<'a> {
    text: Cow<'a, str>,
    style: Style,
}

/// A logical line composed of styled spans with a starting column.
#[derive(Clone, Debug)]
pub struct Line<'a> {
    col: u16,
    spans: Vec<Span<'a>>,
}

impl<'a> Span<'a> {
    /// Creates a plain span.
    #[must_use]
    pub fn plain<T: Into<Cow<'a, str>>>(text: T) -> Self {
        Self {
            text: text.into(),
            style: Style::default(),
        }
    }

    /// Creates a styled span.
    #[must_use]
    pub fn styled<T: Into<Cow<'a, str>>>(text: T, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    /// Returns the span text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_ref()
    }

    /// Returns the span style.
    #[must_use]
    pub fn style(&self) -> Style {
        self.style
    }

    /// Returns the display width of the span.
    #[must_use]
    pub fn width(&self) -> usize {
        width(self.text())
    }
}

impl<'a> Line<'a> {
    /// Creates an empty line starting at `col`.
    #[must_use]
    pub fn new(col: u16) -> Self {
        Self {
            col,
            spans: Vec::new(),
        }
    }

    /// Creates a plain line with sanitized text.
    #[must_use]
    pub fn plain<T: Into<Cow<'a, str>>>(col: u16, text: T) -> Self {
        let mut line = Self::new(col);
        line.push(Span::plain(sanitize(text.into())));

        line
    }

    /// Appends a span to the line.
    pub fn push(&mut self, span: Span<'a>) {
        self.spans.push(span);
    }

    /// Returns the starting column.
    #[must_use]
    pub fn col(&self) -> u16 {
        self.col
    }

    /// Returns the line spans.
    #[must_use]
    pub fn spans(&self) -> &[Span<'a>] {
        &self.spans
    }

    /// Returns the display width of the line.
    #[must_use]
    pub fn width(&self) -> usize {
        self.spans
            .iter()
            .fold(0, |total, span| total.saturating_add(span.width()))
    }
}

/// Returns the display width of `text`.
#[must_use]
pub fn width(text: &str) -> usize {
    text.width()
}

/// Truncates `text` to at most `max_width` display columns.
#[must_use]
pub fn truncate(text: &str, max_width: usize) -> &str {
    let mut used: usize = 0;
    let mut end: usize = 0;

    for (index, character) in text.char_indices() {
        let character_width = character.width().unwrap_or(0);
        let next = used.saturating_add(character_width);
        if next > max_width {
            break;
        }

        used = next;
        end = index.saturating_add(character.len_utf8());
    }

    text.get(..end).unwrap_or(text)
}

/// Shortens `text` to `max_width` display columns by eliding its middle.
///
/// Budgets below three columns truncate the tail instead, leaving no room for
/// the ellipsis.
#[must_use]
pub fn elide(text: &str, max_width: usize) -> Cow<'_, str> {
    if width(text) <= max_width {
        return Cow::Borrowed(text);
    }

    if max_width < 3 {
        return Cow::Borrowed(truncate(text, max_width));
    }

    let budget = max_width.saturating_sub(1);
    let head = budget.div_euclid(2);
    let tail = budget.saturating_sub(head);
    let mut elided = String::with_capacity(text.len());
    elided.push_str(truncate(text, head));
    elided.push('\u{2026}');
    elided.push_str(suffix(text, tail));

    Cow::Owned(elided)
}

/// Replaces tabs and control characters with printable spaces.
#[must_use]
pub fn sanitize(text: Cow<'_, str>) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|character| character == '\t' || character.is_control())
    {
        return text;
    }

    let mut sanitized = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\t' {
            sanitized.push_str("    ");
        } else if character.is_control() {
            sanitized.push(' ');
        } else {
            sanitized.push(character);
        }
    }

    Cow::Owned(sanitized)
}

pub(crate) fn write_span<W: Write>(writer: &mut W, span: &Span<'_>) -> io::Result<()> {
    if span.style() == Style::default() {
        return writer.write_all(span.text().as_bytes());
    }

    write!(writer, "{}", span.style().render())?;
    writer.write_all(span.text().as_bytes())?;
    write!(writer, "{}", span.style().render_reset())
}

fn suffix(text: &str, max_width: usize) -> &str {
    let mut used: usize = 0;
    let mut start = text.len();

    for (index, character) in text.char_indices().rev() {
        let character_width = character.width().unwrap_or(0);
        let next = used.saturating_add(character_width);
        if next > max_width {
            break;
        }

        used = next;
        start = index;
    }

    let suffix = text.get(start..).unwrap_or_default();
    let base = suffix
        .char_indices()
        .find(|&(_, character)| character.width().unwrap_or(0) > 0)
        .map_or(suffix.len(), |(index, _)| index);

    suffix.get(base..).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_wide_characters() {
        // ARRANGE
        let text = "日本";

        // ACT
        let measured = width(text);

        // ASSERT
        assert_eq!(measured, 4);
    }

    #[test]
    fn width_ignores_combining_marks() {
        // ARRANGE
        let text = "e\u{301}";

        // ACT
        let measured = width(text);

        // ASSERT
        assert_eq!(measured, 1);
    }

    #[test]
    fn truncate_keeps_combining_marks_with_their_base() {
        // ARRANGE
        let text = "e\u{301}x";

        // ACT
        let truncated = truncate(text, 1);

        // ASSERT
        assert_eq!(truncated, "e\u{301}");
    }

    #[test]
    fn truncate_never_splits_a_wide_character() {
        // ARRANGE
        let text = "日本語";

        // ACT
        let truncated = truncate(text, 3);

        // ASSERT
        assert_eq!(truncated, "日");
    }

    #[test]
    fn truncate_returns_a_prefix_on_a_boundary() {
        // ARRANGE
        let text = "abcdef";

        // ACT
        let truncated = truncate(text, 3);

        // ASSERT
        assert_eq!(truncated, "abc");
    }

    #[test]
    fn elide_returns_text_that_fits_unchanged() {
        // ARRANGE
        let text = "abc";

        // ACT
        let elided = elide(text, 5);

        // ASSERT
        assert!(matches!(elided, Cow::Borrowed("abc")));
    }

    #[test]
    fn elide_keeps_the_head_and_the_tail() {
        // ARRANGE
        let text = "abcdefghij";

        // ACT
        let elided = elide(text, 5);

        // ASSERT
        assert_eq!(elided, "ab\u{2026}ij");
    }

    #[test]
    fn elide_degrades_for_tiny_budgets() {
        // ARRANGE
        let text = "abcdef";

        // ACT
        let nothing = elide(text, 0);
        let one = elide(text, 1);

        // ASSERT
        assert_eq!(nothing, "");
        assert_eq!(one, "a");
    }

    #[test]
    fn elide_never_splits_a_wide_character() {
        // ARRANGE
        let text = "\u{65e5}\u{672c}\u{8a9e}";

        // ACT
        let elided = elide(text, 4);

        // ASSERT
        assert_eq!(elided, "\u{2026}\u{8a9e}");
    }

    #[test]
    fn elide_drops_an_orphan_combining_mark() {
        // ARRANGE
        let text = "abcdefg\u{65e5}\u{301}";

        // ACT
        let elided = elide(text, 3);

        // ASSERT
        assert_eq!(elided, "a\u{2026}");
    }

    #[test]
    fn elide_never_exceeds_the_budget() {
        // ARRANGE
        let text = "e\u{301}\u{65e5}\u{672c}\u{8a9e}abcdef";

        // ACT
        let widths: Vec<usize> = (0..=12).map(|max| width(&elide(text, max))).collect();

        // ASSERT
        for (max, measured) in widths.into_iter().enumerate() {
            assert!(measured <= max, "max {max} produced {measured}");
        }
    }

    #[test]
    fn sanitize_replaces_tabs_and_controls() {
        // ARRANGE
        let text = Cow::Borrowed("a\tb\u{1}c");

        // ACT
        let sanitized = sanitize(text);

        // ASSERT
        assert_eq!(sanitized, "a    b c");
    }

    #[test]
    fn sanitize_borrows_clean_text() {
        // ARRANGE
        let text = Cow::Borrowed("clean");

        // ACT
        let sanitized = sanitize(text);

        // ASSERT
        assert!(matches!(sanitized, Cow::Borrowed("clean")));
    }

    #[test]
    fn line_plain_sanitizes_user_text() {
        // ARRANGE
        let text = "a\tb";

        // ACT
        let line = Line::plain(0, text);

        // ASSERT
        assert_eq!(line.width(), 6);
        assert_eq!(line.spans().first().map(Span::text), Some("a    b"));
    }

    #[test]
    fn write_span_emits_plain_text_without_escapes() {
        // ARRANGE
        let mut output = Vec::new();
        let span = Span::plain("plain");

        // ACT
        write_span(&mut output, &span).unwrap();

        // ASSERT
        assert_eq!(output, b"plain".to_vec());
    }
}
