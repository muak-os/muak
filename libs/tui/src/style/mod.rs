//! Semantic colors and text styling.

pub(crate) mod color;
pub mod semantic;

use std::sync::OnceLock;

/// Text style from the ANSI styling vocabulary.
pub type Style = anstyle::Style;

/// Foreground or background color.
pub type Color = anstyle::Color;

/// Color from the 4-bit ANSI palette.
pub type AnsiColor = anstyle::AnsiColor;

/// Color and role policy.
pub type Theme = semantic::Theme;

/// Semantic role assigned to a piece of text.
pub type Role = semantic::Role;

/// Output stream used to decide whether color is available.
pub type Stream = semantic::Stream;

/// Styled text ready to be displayed.
pub type Styled<'a> = semantic::Styled<'a>;

/// Creates a foreground color style.
#[must_use]
pub const fn fg(color: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(color)))
}

/// Creates a bold foreground color style.
#[must_use]
pub const fn bold_fg(color: AnsiColor) -> Style {
    fg(color).bold()
}

/// Creates a bold style.
#[must_use]
pub const fn bold() -> Style {
    Style::new().bold()
}

/// Creates a dim style.
#[must_use]
pub const fn dim() -> Style {
    Style::new().dimmed()
}

/// Returns the standard output theme.
#[must_use]
pub fn stdout() -> Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();

    *THEME.get_or_init(|| Theme::detect(Stream::Stdout))
}

/// Returns the standard error theme.
#[must_use]
pub fn stderr() -> Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();

    *THEME.get_or_init(|| Theme::detect(Stream::Stderr))
}

/// Paints `text` as a success on standard output.
#[must_use]
pub fn success(text: &str) -> Styled<'_> {
    stdout().success(text)
}

/// Paints `text` as an error on standard output.
#[must_use]
pub fn error(text: &str) -> Styled<'_> {
    stdout().error(text)
}

/// Paints `text` as error body text on standard output.
#[must_use]
pub fn error_text(text: &str) -> Styled<'_> {
    stdout().error_text(text)
}

/// Paints `text` as a warning on standard output.
#[must_use]
pub fn warn(text: &str) -> Styled<'_> {
    stdout().warn(text)
}

/// Paints `text` as information on standard output.
#[must_use]
pub fn info(text: &str) -> Styled<'_> {
    stdout().info(text)
}

/// Paints `text` as muted on standard output.
#[must_use]
pub fn muted(text: &str) -> Styled<'_> {
    stdout().muted(text)
}

/// Paints `text` as an accent on standard output.
#[must_use]
pub fn accent(text: &str) -> Styled<'_> {
    stdout().accent(text)
}

/// Paints `text` as a header on standard output.
#[must_use]
pub fn header(text: &str) -> Styled<'_> {
    stdout().header(text)
}

/// Paints `text` as a label on standard output.
#[must_use]
pub fn label(text: &str) -> Styled<'_> {
    stdout().label(text)
}

/// Paints `text` as a highlighted value on standard output.
#[must_use]
pub fn highlight(text: &str) -> Styled<'_> {
    stdout().highlight(text)
}

/// Paints `text` as a positive value on standard output.
#[must_use]
pub fn positive(text: &str) -> Styled<'_> {
    stdout().positive(text)
}

/// Paints `text` as a negative value on standard output.
#[must_use]
pub fn negative(text: &str) -> Styled<'_> {
    stdout().negative(text)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn plain_theme_discards_roles() {
        // ARRANGE
        let theme = Theme::plain();

        // ACT
        let rendered = theme.success("ok").to_string();

        // ASSERT
        assert_eq!(rendered, "ok");
    }

    #[test]
    fn colored_theme_emits_ansi() {
        // ARRANGE
        let theme = Theme::colored();

        // ACT
        let rendered = theme.error("failed").to_string();

        // ASSERT
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains("failed"));
    }

    #[test]
    fn no_color_disables_color() {
        // ARRANGE
        let no_color = OsStr::new("1");

        // ACT
        let enabled = color::should_color(true, Some(no_color), None, None, None);

        // ASSERT
        assert!(!enabled);
    }

    #[test]
    fn empty_no_color_keeps_color() {
        // ARRANGE
        let no_color = OsStr::new("");

        // ACT
        let enabled = color::should_color(true, Some(no_color), None, None, None);

        // ASSERT
        assert!(enabled);
    }

    #[test]
    fn clicolor_zero_disables_color() {
        // ARRANGE
        let clicolor = OsStr::new("0");

        // ACT
        let enabled = color::should_color(true, None, Some(clicolor), None, None);

        // ASSERT
        assert!(!enabled);
    }

    #[test]
    fn clicolor_force_enables_color_for_pipes() {
        // ARRANGE
        let force = OsStr::new("1");

        // ACT
        let enabled = color::should_color(false, None, None, Some(force), None);

        // ASSERT
        assert!(enabled);
    }

    #[test]
    fn dumb_terminal_disables_color() {
        // ARRANGE
        let term = OsStr::new("dumb");

        // ACT
        let enabled = color::should_color(true, None, None, None, Some(term));

        // ASSERT
        assert!(!enabled);
    }

    #[test]
    fn pipe_without_overrides_disables_color() {
        // ARRANGE
        let terminal = false;

        // ACT
        let enabled = color::should_color(terminal, None, None, None, None);

        // ASSERT
        assert!(!enabled);
    }

    #[test]
    fn role_styles_match_the_palette() {
        // ARRANGE
        let theme = Theme::colored();

        // ACT
        let success = theme.style(Role::Success);
        let muted = theme.style(Role::Muted);

        // ASSERT
        assert_eq!(success, bold_fg(AnsiColor::Green));
        assert_eq!(muted, dim());
    }
}
