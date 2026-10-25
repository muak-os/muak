//! Rich and plain glyph sets.

/// Symbol set used by live widgets, tables, and summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    /// Completed operation marker.
    pub success: &'static str,
    /// Failed operation marker.
    pub failure: &'static str,
    /// Pending operation marker.
    pub pending: &'static str,
    /// Spinner animation frames.
    pub spinner: &'static [&'static str],
    /// Filled progress bar cell.
    pub bar_filled: &'static str,
    /// Empty progress bar cell.
    pub bar_empty: &'static str,
    /// Horizontal rule.
    pub rule: &'static str,
    /// List bullet.
    pub bullet: &'static str,
    /// Direction arrow.
    pub arrow: &'static str,
}

impl Glyphs {
    /// Returns the Unicode glyph set for rich output.
    #[must_use]
    pub const fn rich() -> Self {
        Self {
            success: "\u{2713}",
            failure: "\u{2717}",
            pending: "\u{b7}",
            spinner: &[
                "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
                "\u{2827}",
            ],
            bar_filled: "\u{2588}",
            bar_empty: "\u{2591}",
            rule: "\u{2500}",
            bullet: "\u{251c}\u{2500}",
            arrow: "\u{2192}",
        }
    }

    /// Returns the ASCII glyph set for plain output.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            success: "[ok]",
            failure: "[x]",
            pending: ".",
            spinner: &["|", "/", "-", "\\"],
            bar_filled: "#",
            bar_empty: "-",
            rule: "-",
            bullet: "|-",
            arrow: "->",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_glyphs_use_unicode_markers() {
        // ARRANGE
        let glyphs = Glyphs::rich();

        // ACT
        let success = glyphs.success;

        // ASSERT
        assert_eq!(success, "\u{2713}");
        assert_eq!(glyphs.failure, "\u{2717}");
        assert_eq!(glyphs.spinner.len(), 8);
    }

    #[test]
    fn plain_glyphs_are_ascii() {
        // ARRANGE
        let glyphs = Glyphs::plain();

        // ACT
        let all = [
            glyphs.success,
            glyphs.failure,
            glyphs.pending,
            glyphs.bar_filled,
            glyphs.bar_empty,
            glyphs.rule,
            glyphs.bullet,
            glyphs.arrow,
        ];

        // ASSERT
        assert!(all.iter().all(|glyph| glyph.is_ascii()));
        assert!(glyphs.spinner.iter().all(|glyph| glyph.is_ascii()));
    }
}
