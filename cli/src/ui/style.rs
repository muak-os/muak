//! Semantic color theme for consistent CLI output.

use core::fmt;

use crossterm::style::{StyledContent, Stylize as _};

/// Whether color output is enabled for this session.
fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// A wrapper that can hold either styled or plain text.
#[derive(Copy, Clone)]
pub enum Styled<'a> {
    Colored(StyledContent<&'a str>),
    Plain(&'a str),
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Styled::Colored(text) => text.fmt(f),
            Styled::Plain(text) => text.fmt(f),
        }
    }
}

fn make_success(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.green().bold())
    } else {
        Styled::Plain(text)
    }
}

fn make_error(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.red().bold())
    } else {
        Styled::Plain(text)
    }
}

fn make_error_text(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.red())
    } else {
        Styled::Plain(text)
    }
}

fn make_warn(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.yellow())
    } else {
        Styled::Plain(text)
    }
}

fn make_info(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.blue())
    } else {
        Styled::Plain(text)
    }
}

fn make_muted(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.dim())
    } else {
        Styled::Plain(text)
    }
}

fn make_accent(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.cyan())
    } else {
        Styled::Plain(text)
    }
}

fn make_header(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.green().bold())
    } else {
        Styled::Plain(text)
    }
}

fn make_label(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.bold())
    } else {
        Styled::Plain(text)
    }
}

fn make_highlight(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.yellow())
    } else {
        Styled::Plain(text)
    }
}

fn make_positive(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.green())
    } else {
        Styled::Plain(text)
    }
}

fn make_negative(text: &str, color: bool) -> Styled<'_> {
    if color {
        Styled::Colored(text.red())
    } else {
        Styled::Plain(text)
    }
}

/// Success messages (green bold). Use for completed operations.
#[must_use]
pub fn success(text: &str) -> Styled<'_> {
    make_success(text, color_enabled())
}

/// Error messages (red bold). Use for failures and error labels.
#[must_use]
pub fn error(text: &str) -> Styled<'_> {
    make_error(text, color_enabled())
}

/// Error body text (red, not bold). Use for error descriptions.
#[must_use]
pub fn error_text(text: &str) -> Styled<'_> {
    make_error_text(text, color_enabled())
}

/// Warning / caution messages (yellow).
#[must_use]
pub fn warn(text: &str) -> Styled<'_> {
    make_warn(text, color_enabled())
}

/// In-progress / informational messages (blue).
#[must_use]
pub fn info(text: &str) -> Styled<'_> {
    make_info(text, color_enabled())
}

/// De-emphasized / secondary text (dim).
#[must_use]
pub fn muted(text: &str) -> Styled<'_> {
    make_muted(text, color_enabled())
}

/// Accent color for counts, identifiers, endpoints (cyan).
#[must_use]
pub fn accent(text: &str) -> Styled<'_> {
    make_accent(text, color_enabled())
}

/// Table / section headers (green bold).
#[must_use]
pub fn header(text: &str) -> Styled<'_> {
    make_header(text, color_enabled())
}

/// Key labels like "Context:", "Endpoint:" (bold).
#[must_use]
pub fn label(text: &str) -> Styled<'_> {
    make_label(text, color_enabled())
}

/// Highlighted data values -- fingerprints, pending items (yellow).
#[must_use]
pub fn highlight(text: &str) -> Styled<'_> {
    make_highlight(text, color_enabled())
}

/// Active / positive data values (green, not bold).
#[must_use]
pub fn positive(text: &str) -> Styled<'_> {
    make_positive(text, color_enabled())
}

/// Negative / danger data values (red, not bold).
#[must_use]
pub fn negative(text: &str) -> Styled<'_> {
    make_negative(text, color_enabled())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_str(styled: Styled<'_>) -> String {
        match styled {
            Styled::Plain(text) => text.to_owned(),
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
