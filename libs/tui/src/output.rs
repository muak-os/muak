//! Output capability model.

use std::ffi::{OsStr, OsString};

use crate::glyphs::Glyphs;
use crate::size::{self, Size};
use crate::style::{self, Stream, Theme};

/// Rendering mode for a CLI stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Human-facing output with color, glyphs, and animation.
    Rich,
    /// Stable line-oriented output without color or animation.
    Plain,
    /// Machine-facing output that suppresses progress and success lines.
    Quiet,
}

/// Explicit output flags supplied by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// Suppresses progress and decorative success lines.
    pub quiet: bool,
    /// Disables color while keeping the detected mode.
    pub no_color: bool,
    /// Enables per-item detail in rich and plain modes.
    pub verbose: bool,
}

/// Environment and terminal inputs used to detect output capabilities.
#[derive(Clone, Debug, Default)]
pub struct DetectInput {
    /// Whether the target stream is attached to a terminal.
    pub terminal: bool,
    /// Value of the `TERM` environment variable.
    pub term: Option<OsString>,
    /// Value of the `CI` environment variable.
    pub ci: Option<OsString>,
    /// Value of the `NO_COLOR` environment variable.
    pub no_color: Option<OsString>,
    /// Value of the `CLICOLOR` environment variable.
    pub clicolor: Option<OsString>,
    /// Value of the `CLICOLOR_FORCE` environment variable.
    pub clicolor_force: Option<OsString>,
    /// Value of the `FORCE_COLOR` environment variable.
    pub force_color: Option<OsString>,
}

/// Resolved output capabilities for one stream.
#[derive(Clone, Copy, Debug)]
pub struct Output {
    /// Rendering mode.
    pub mode: Mode,
    /// Color and role policy.
    pub theme: Theme,
    /// Symbol set matching the mode.
    pub glyphs: Glyphs,
    /// Terminal surface dimensions.
    pub size: Size,
    /// Whether per-item detail is enabled.
    pub verbose: bool,
}

impl DetectInput {
    /// Captures detection inputs for `stream` from the process environment.
    #[must_use]
    pub fn capture(stream: Stream) -> Self {
        Self {
            terminal: stream.is_terminal(),
            term: std::env::var_os("TERM"),
            ci: std::env::var_os("CI"),
            no_color: std::env::var_os("NO_COLOR"),
            clicolor: std::env::var_os("CLICOLOR"),
            clicolor_force: std::env::var_os("CLICOLOR_FORCE"),
            force_color: std::env::var_os("FORCE_COLOR"),
        }
    }

    fn is_ci(&self) -> bool {
        self.ci.as_deref().is_some_and(|value| !value.is_empty())
    }

    fn is_dumb(&self) -> bool {
        self.term.as_deref() == Some(OsStr::new("dumb"))
    }
}

impl Output {
    /// Detects output capabilities for `stream` with explicit `flags`.
    #[must_use]
    pub fn detect(stream: Stream, flags: Flags) -> Self {
        Self::from_input(stream, flags, &DetectInput::capture(stream))
    }

    /// Resolves output capabilities from explicit detection inputs.
    #[must_use]
    pub fn from_input(stream: Stream, flags: Flags, input: &DetectInput) -> Self {
        let mode = if flags.quiet {
            Mode::Quiet
        } else if input.terminal && !input.is_dumb() && !input.is_ci() {
            Mode::Rich
        } else {
            Mode::Plain
        };
        let theme = if color_enabled(input, flags, mode) {
            Theme::colored()
        } else {
            Theme::plain()
        };
        let glyphs = match mode {
            Mode::Rich => Glyphs::rich(),
            Mode::Plain | Mode::Quiet => Glyphs::plain(),
        };

        Self {
            mode,
            theme,
            glyphs,
            size: size::surface(stream),
            verbose: flags.verbose && !flags.quiet,
        }
    }

    /// Returns whether the output is quiet.
    #[must_use]
    pub const fn is_quiet(self) -> bool {
        matches!(self.mode, Mode::Quiet)
    }

    /// Returns whether the output is rich.
    #[must_use]
    pub const fn is_rich(self) -> bool {
        matches!(self.mode, Mode::Rich)
    }

    /// Returns whether the output is plain.
    #[must_use]
    pub const fn is_plain(self) -> bool {
        matches!(self.mode, Mode::Plain)
    }

    /// Returns whether live widgets may animate.
    #[must_use]
    pub const fn animates(self) -> bool {
        self.is_rich()
    }
}

fn color_enabled(input: &DetectInput, flags: Flags, mode: Mode) -> bool {
    if flags.no_color || matches!(mode, Mode::Quiet) {
        return false;
    }

    let force = input
        .clicolor_force
        .as_deref()
        .or(input.force_color.as_deref());
    let terminal = matches!(mode, Mode::Rich);

    style::color::should_color(
        terminal,
        input.no_color.as_deref(),
        input.clicolor.as_deref(),
        force,
        input.term.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn input(terminal: bool) -> DetectInput {
        DetectInput {
            terminal,
            ..DetectInput::default()
        }
    }

    #[test]
    fn smart_tty_is_rich_with_color() {
        // ARRANGE
        let input = input(true);

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Rich);
        assert!(output.animates());
        assert_eq!(output.glyphs, Glyphs::rich());
        assert!(output.theme.success("ok").to_string().contains('\u{1b}'));
    }

    #[test]
    fn pipe_is_plain_without_color() {
        // ARRANGE
        let input = input(false);

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
        assert_eq!(output.glyphs, Glyphs::plain());
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn dumb_terminal_is_plain_even_on_a_tty() {
        // ARRANGE
        let mut input = input(true);
        input.term = Some(OsString::from("dumb"));

        // ACT
        let output = Output::from_input(Stream::Stderr, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
    }

    #[test]
    fn ci_is_plain_even_on_a_tty() {
        // ARRANGE
        let mut input = input(true);
        input.ci = Some(OsString::from("1"));

        // ACT
        let output = Output::from_input(Stream::Stderr, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn ci_color_requires_a_force_override() {
        // ARRANGE
        let mut input = input(true);
        input.ci = Some(OsString::from("1"));
        input.force_color = Some(OsString::from("1"));

        // ACT
        let output = Output::from_input(Stream::Stderr, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
        assert!(output.theme.success("ok").to_string().contains('\u{1b}'));
    }

    #[test]
    fn quiet_wins_over_rich_detection() {
        // ARRANGE
        let input = input(true);
        let flags = Flags {
            quiet: true,
            ..Flags::default()
        };

        // ACT
        let output = Output::from_input(Stream::Stderr, flags, &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Quiet);
        assert!(output.is_quiet());
        assert!(!output.animates());
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn no_color_keeps_the_rich_mode() {
        // ARRANGE
        let input = input(true);
        let flags = Flags {
            no_color: true,
            ..Flags::default()
        };

        // ACT
        let output = Output::from_input(Stream::Stdout, flags, &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Rich);
        assert_eq!(output.glyphs, Glyphs::rich());
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn no_color_env_keeps_rich_mode_without_color() {
        // ARRANGE
        let mut input = input(true);
        input.no_color = Some(OsString::from("1"));

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Rich);
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn clicolor_zero_disables_color() {
        // ARRANGE
        let mut input = input(true);
        input.clicolor = Some(OsString::from("0"));

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Rich);
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn clicolor_force_enables_color_for_pipes() {
        // ARRANGE
        let mut input = input(false);
        input.clicolor_force = Some(OsString::from("1"));

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
        assert!(output.theme.success("ok").to_string().contains('\u{1b}'));
    }

    #[test]
    fn force_color_enables_color_for_pipes() {
        // ARRANGE
        let mut input = input(false);
        input.force_color = Some(OsString::from("1"));

        // ACT
        let output = Output::from_input(Stream::Stdout, Flags::default(), &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Plain);
        assert!(output.theme.success("ok").to_string().contains('\u{1b}'));
    }

    #[test]
    fn no_color_flag_disables_color_in_rich_mode() {
        // ARRANGE
        let input = input(true);
        let flags = Flags {
            no_color: true,
            ..Flags::default()
        };

        // ACT
        let output = Output::from_input(Stream::Stdout, flags, &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Rich);
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn quiet_output_uses_plain_glyphs_and_no_color() {
        // ARRANGE
        let input = input(true);
        let flags = Flags {
            quiet: true,
            ..Flags::default()
        };

        // ACT
        let output = Output::from_input(Stream::Stderr, flags, &input);

        // ASSERT
        assert_eq!(output.mode, Mode::Quiet);
        assert_eq!(output.glyphs, Glyphs::plain());
        assert_eq!(output.theme.success("ok").to_string(), "ok");
    }

    #[test]
    fn quiet_disables_verbose_detail() {
        // ARRANGE
        let input = input(true);
        let flags = Flags {
            quiet: true,
            verbose: true,
            ..Flags::default()
        };

        // ACT
        let output = Output::from_input(Stream::Stdout, flags, &input);

        // ASSERT
        assert!(!output.verbose);
    }
}
