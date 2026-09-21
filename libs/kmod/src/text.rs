//! Byte-span storage over a single text arena.

/// A byte range into an arena `String`, replacing one heap allocation per field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) len: usize,
}

/// Slices `span` out of `text`, returning an empty string when out of bounds.
pub(crate) fn slice(text: &str, span: Span) -> &str {
    text.get(span.start..span.start.saturating_add(span.len))
        .unwrap_or_default()
}

/// Creates a span for the trimmed portion of `raw`, which starts at `base` in the arena.
pub(crate) fn span_at(base: usize, raw: &str) -> Span {
    let lead = raw.len().saturating_sub(raw.trim_start().len());
    Span {
        start: base.saturating_add(lead),
        len: raw.trim().len(),
    }
}

/// Calls `callback` with every `'\n'`-separated line of `text` and its byte offset.
pub(crate) fn for_each_line(text: &str, mut callback: impl FnMut(&str, usize)) {
    let mut offset = 0;
    for line in text.split('\n') {
        callback(line, offset);
        offset = offset.saturating_add(line.len()).saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_returns_text_at_span() {
        // ARRANGE
        let text = "alias pci:v1234 igc\nalias usb:v5678 usbhid";
        let span = Span { start: 6, len: 9 };

        // ACT
        let sliced = slice(text, span);

        // ASSERT
        assert_eq!(sliced, "pci:v1234");
    }

    #[test]
    fn slice_out_of_bounds_is_empty() {
        // ARRANGE
        let span = Span { start: 100, len: 4 };

        // ACT
        let sliced = slice("text", span);

        // ASSERT
        assert_eq!(sliced, "");
    }

    #[test]
    fn span_at_trims_whitespace() {
        // ARRANGE
        let text = "  pci:pattern  ";

        // ACT
        let span = span_at(0, text);

        // ASSERT
        assert_eq!(slice(text, span), "pci:pattern");
    }

    #[test]
    fn for_each_line_reports_offsets() {
        // ARRANGE
        let text = "ab\ncd\n";
        let mut lines = Vec::new();

        // ACT
        for_each_line(text, |line, offset| lines.push((line.to_owned(), offset)));

        // ASSERT
        assert_eq!(
            lines,
            [
                ("ab".to_owned(), 0),
                ("cd".to_owned(), 3),
                (String::new(), 6)
            ]
        );
    }
}
