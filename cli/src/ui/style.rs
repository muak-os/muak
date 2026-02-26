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

/// Success messages (green bold). Use for completed operations.
pub fn success(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.green().bold())
    } else {
        Styled::Plain(s)
    }
}

/// Error messages (red bold). Use for failures and error labels.
pub fn error(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.red().bold())
    } else {
        Styled::Plain(s)
    }
}

/// Error body text (red, not bold). Use for error descriptions.
pub fn error_text(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.red())
    } else {
        Styled::Plain(s)
    }
}

/// Warning / caution messages (yellow).
pub fn warn(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.yellow())
    } else {
        Styled::Plain(s)
    }
}

/// In-progress / informational messages (blue).
pub fn info(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.blue())
    } else {
        Styled::Plain(s)
    }
}

/// De-emphasized / secondary text (dim).
pub fn muted(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.dim())
    } else {
        Styled::Plain(s)
    }
}

/// Accent color for counts, identifiers, endpoints (cyan).
pub fn accent(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.cyan())
    } else {
        Styled::Plain(s)
    }
}

/// Table / section headers (green bold).
pub fn header(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.green().bold())
    } else {
        Styled::Plain(s)
    }
}

/// Key labels like "Context:", "Endpoint:" (bold).
pub fn label(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.bold())
    } else {
        Styled::Plain(s)
    }
}

/// Highlighted data values -- fingerprints, pending items (yellow).
pub fn highlight(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.yellow())
    } else {
        Styled::Plain(s)
    }
}

/// Active / positive data values (green, not bold).
pub fn positive(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.green())
    } else {
        Styled::Plain(s)
    }
}

/// Negative / danger data values (red, not bold).
pub fn negative(s: &str) -> Styled<'_> {
    if color_enabled() {
        Styled::Colored(s.red())
    } else {
        Styled::Plain(s)
    }
}
