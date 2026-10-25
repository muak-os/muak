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
