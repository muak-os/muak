//! Semantic color theme for consistent CLI output.

use std::fmt;

use crossterm::style::{StyledContent, Stylize};

/// Whether color output is enabled for this session.
fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// A wrapper that can hold either styled or plain text.
pub enum Styled<'a> {
    Colored(StyledContent<&'a str>),
    Plain(&'a str),
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Styled::Colored(s) => s.fmt(f),
            Styled::Plain(s) => s.fmt(f),
        }
    }
}

fn make_success(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.green().bold())
    } else {
        Styled::Plain(s)
    }
}

fn make_error(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.red().bold())
    } else {
        Styled::Plain(s)
    }
}

fn make_error_text(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.red())
    } else {
        Styled::Plain(s)
    }
}

fn make_warn(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.yellow())
    } else {
        Styled::Plain(s)
    }
}

fn make_info(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.blue())
    } else {
        Styled::Plain(s)
    }
}

fn make_muted(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.dim())
    } else {
        Styled::Plain(s)
    }
}

fn make_accent(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.cyan())
    } else {
        Styled::Plain(s)
    }
}

fn make_header(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.green().bold())
    } else {
        Styled::Plain(s)
    }
}

fn make_label(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.bold())
    } else {
        Styled::Plain(s)
    }
}

fn make_highlight(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.yellow())
    } else {
        Styled::Plain(s)
    }
}

fn make_positive(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.green())
    } else {
        Styled::Plain(s)
    }
}

fn make_negative(s: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(s.red())
    } else {
        Styled::Plain(s)
    }
}

/// Success messages (green bold). Use for completed operations.
pub fn success(s: &str) -> Styled<'_> {
    make_success(s, color_enabled())
}

/// Error messages (red bold). Use for failures and error labels.
pub fn error(s: &str) -> Styled<'_> {
    make_error(s, color_enabled())
}

/// Error body text (red, not bold). Use for error descriptions.
pub fn error_text(s: &str) -> Styled<'_> {
    make_error_text(s, color_enabled())
}

/// Warning / caution messages (yellow).
pub fn warn(s: &str) -> Styled<'_> {
    make_warn(s, color_enabled())
}

/// In-progress / informational messages (blue).
pub fn info(s: &str) -> Styled<'_> {
    make_info(s, color_enabled())
}

/// De-emphasized / secondary text (dim).
pub fn muted(s: &str) -> Styled<'_> {
    make_muted(s, color_enabled())
}

/// Accent color for counts, identifiers, endpoints (cyan).
pub fn accent(s: &str) -> Styled<'_> {
    make_accent(s, color_enabled())
}

/// Table / section headers (green bold).
pub fn header(s: &str) -> Styled<'_> {
    make_header(s, color_enabled())
}

/// Key labels like "Context:", "Endpoint:" (bold).
pub fn label(s: &str) -> Styled<'_> {
    make_label(s, color_enabled())
}

/// Highlighted data values -- fingerprints, pending items (yellow).
pub fn highlight(s: &str) -> Styled<'_> {
    make_highlight(s, color_enabled())
}

/// Active / positive data values (green, not bold).
pub fn positive(s: &str) -> Styled<'_> {
    make_positive(s, color_enabled())
}

/// Negative / danger data values (red, not bold).
pub fn negative(s: &str) -> Styled<'_> {
    make_negative(s, color_enabled())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_str(styled: Styled<'_>) -> String {
        match styled {
            Styled::Plain(s) => s.to_string(),
            Styled::Colored(_) => panic!("expected Plain variant"),
        }
    }

    fn colored_display(styled: Styled<'_>) -> String {
        match styled {
            Styled::Colored(_) => styled.to_string(),
            Styled::Plain(_) => panic!("expected Colored variant"),
        }
    }

    // Plain path: every function returns the original string unchanged.
    #[test]
    fn plain_success() {
        // ARRANGE & ACT
        let result = plain_str(make_success("ok", false));

        // ASSERT
        assert_eq!(result, "ok");
    }

    #[test]
    fn plain_error() {
        assert_eq!(plain_str(make_error("err", false)), "err");
    }

    #[test]
    fn plain_error_text() {
        assert_eq!(plain_str(make_error_text("desc", false)), "desc");
    }

    #[test]
    fn plain_warn() {
        assert_eq!(plain_str(make_warn("careful", false)), "careful");
    }

    #[test]
    fn plain_info() {
        assert_eq!(plain_str(make_info("loading", false)), "loading");
    }

    #[test]
    fn plain_muted() {
        assert_eq!(plain_str(make_muted("dim", false)), "dim");
    }

    #[test]
    fn plain_accent() {
        assert_eq!(plain_str(make_accent("cyan", false)), "cyan");
    }

    #[test]
    fn plain_header() {
        assert_eq!(plain_str(make_header("NAME", false)), "NAME");
    }

    #[test]
    fn plain_label() {
        assert_eq!(plain_str(make_label("Context:", false)), "Context:");
    }

    #[test]
    fn plain_highlight() {
        assert_eq!(plain_str(make_highlight("abc123", false)), "abc123");
    }

    #[test]
    fn plain_positive() {
        assert_eq!(plain_str(make_positive("active", false)), "active");
    }

    #[test]
    fn plain_negative() {
        assert_eq!(plain_str(make_negative("failed", false)), "failed");
    }

    // Colored path: Display output must contain the original string.
    #[test]
    fn colored_success_contains_text() {
        // ARRANGE & ACT
        let result = colored_display(make_success("ok", true));

        // ASSERT
        assert!(result.contains("ok"));
    }

    #[test]
    fn colored_error_contains_text() {
        assert!(colored_display(make_error("err", true)).contains("err"));
    }

    #[test]
    fn colored_warn_contains_text() {
        assert!(colored_display(make_warn("careful", true)).contains("careful"));
    }

    #[test]
    fn colored_info_contains_text() {
        assert!(colored_display(make_info("loading", true)).contains("loading"));
    }

    #[test]
    fn colored_muted_contains_text() {
        assert!(colored_display(make_muted("dim", true)).contains("dim"));
    }

    #[test]
    fn colored_accent_contains_text() {
        assert!(colored_display(make_accent("cyan", true)).contains("cyan"));
    }

    #[test]
    fn colored_header_contains_text() {
        assert!(colored_display(make_header("NAME", true)).contains("NAME"));
    }

    #[test]
    fn colored_label_contains_text() {
        assert!(colored_display(make_label("Context:", true)).contains("Context:"));
    }

    #[test]
    fn colored_highlight_contains_text() {
        assert!(colored_display(make_highlight("abc123", true)).contains("abc123"));
    }

    #[test]
    fn colored_positive_contains_text() {
        assert!(colored_display(make_positive("active", true)).contains("active"));
    }

    #[test]
    fn colored_negative_contains_text() {
        assert!(colored_display(make_negative("failed", true)).contains("failed"));
    }

    // Display impl: plain variant formats to the raw string.
    #[test]
    fn display_plain_equals_input() {
        // ARRANGE & ACT
        let result = make_success("hello", false).to_string();

        // ASSERT
        assert_eq!(result, "hello");
    }
}
