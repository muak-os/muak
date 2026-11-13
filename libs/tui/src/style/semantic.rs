//! Semantic roles and themes.

use core::fmt;
use std::io::IsTerminal as _;

use super::color::{role_style, should_color};
use crate::style::Style;

/// Semantic role assigned to a piece of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Completed operations.
    Success,
    /// Failures and error labels.
    Error,
    /// Error descriptions.
    ErrorText,
    /// Warnings and cautions.
    Warn,
    /// Informational messages.
    Info,
    /// De-emphasized text.
    Muted,
    /// Counts, identifiers, and endpoints.
    Accent,
    /// Table and section headers.
    Header,
    /// Key labels.
    Label,
    /// Highlighted data values.
    Highlight,
    /// Active and positive data values.
    Positive,
    /// Negative and dangerous data values.
    Negative,
}

/// Output stream used to decide whether color is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Styled text ready to be displayed.
#[derive(Clone, Copy, Debug)]
pub struct Styled<'a> {
    text: &'a str,
    style: Style,
}

/// Color and role policy.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    enabled: bool,
}

impl Stream {
    /// Returns whether this stream is attached to a terminal.
    #[must_use = "callers must branch on whether output is a terminal"]
    pub fn is_terminal(self) -> bool {
        match self {
            Self::Stdout => std::io::stdout().is_terminal(),
            Self::Stderr => std::io::stderr().is_terminal(),
        }
    }
}

impl Theme {
    /// Creates a theme that always emits color.
    #[must_use = "dropping it discards the theme"]
    pub const fn colored() -> Self {
        Self { enabled: true }
    }

    /// Creates a theme that never emits color.
    #[must_use = "dropping it discards the theme"]
    pub const fn plain() -> Self {
        Self { enabled: false }
    }

    /// Detects the color policy for `stream` from the environment.
    #[must_use = "dropping it discards the detected theme"]
    pub fn detect(stream: Stream) -> Self {
        let no_color = std::env::var_os("NO_COLOR");
        let clicolor = std::env::var_os("CLICOLOR");
        let clicolor_force = std::env::var_os("CLICOLOR_FORCE");
        let term = std::env::var_os("TERM");
        let enabled = should_color(
            stream.is_terminal(),
            no_color.as_deref(),
            clicolor.as_deref(),
            clicolor_force.as_deref(),
            term.as_deref(),
        );

        Self { enabled }
    }

    /// Returns the style for `role`.
    #[must_use = "callers need the style to paint text"]
    pub fn style(self, role: Role) -> Style {
        if self.enabled {
            role_style(role)
        } else {
            Style::default()
        }
    }

    /// Paints `text` with the style for `role`.
    #[must_use = "callers need the styled text to print it"]
    pub fn paint(self, text: &str, role: Role) -> Styled<'_> {
        Styled {
            text,
            style: self.style(role),
        }
    }

    /// Paints `text` as a success.
    #[must_use = "callers need the styled text to print it"]
    pub fn success(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Success)
    }

    /// Paints `text` as an error.
    #[must_use = "callers need the styled text to print it"]
    pub fn error(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Error)
    }

    /// Paints `text` as error body text.
    #[must_use = "callers need the styled text to print it"]
    pub fn error_text(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::ErrorText)
    }

    /// Paints `text` as a warning.
    #[must_use = "callers need the styled text to print it"]
    pub fn warn(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Warn)
    }

    /// Paints `text` as information.
    #[must_use = "callers need the styled text to print it"]
    pub fn info(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Info)
    }

    /// Paints `text` as muted.
    #[must_use = "callers need the styled text to print it"]
    pub fn muted(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Muted)
    }

    /// Paints `text` as an accent.
    #[must_use = "callers need the styled text to print it"]
    pub fn accent(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Accent)
    }

    /// Paints `text` as a header.
    #[must_use = "callers need the styled text to print it"]
    pub fn header(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Header)
    }

    /// Paints `text` as a label.
    #[must_use = "callers need the styled text to print it"]
    pub fn label(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Label)
    }

    /// Paints `text` as a highlighted value.
    #[must_use = "callers need the styled text to print it"]
    pub fn highlight(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Highlight)
    }

    /// Paints `text` as a positive value.
    #[must_use = "callers need the styled text to print it"]
    pub fn positive(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Positive)
    }

    /// Paints `text` as a negative value.
    #[must_use = "callers need the styled text to print it"]
    pub fn negative(self, text: &str) -> Styled<'_> {
        self.paint(text, Role::Negative)
    }
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.style.render())?;
        f.write_str(self.text)?;
        write!(f, "{}", self.style.render_reset())
    }
}
