//! Console capabilities for the CLI.

use std::io;

use tui::live::{Spinner, Steps};
use tui::output::{Flags, Output};
use tui::screen::Ansi;
use tui::style::{Role, Stream, Style, Styled};
use tui::table::Table;

use crate::OutputFlags;

/// Resolved standard output and standard error for one CLI run.
pub(crate) struct Console {
    stdout: Output,
    stderr: Output,
}

impl Console {
    /// Detects both streams from the global output `flags`.
    pub(crate) fn detect(flags: OutputFlags) -> Self {
        let flags = Flags {
            quiet: flags.quiet,
            no_color: flags.no_color,
            verbose: flags.verbose,
        };

        Self {
            stdout: Output::detect(Stream::Stdout, flags),
            stderr: Output::detect(Stream::Stderr, flags),
        }
    }

    /// Prints a success line to standard output unless quiet.
    pub(crate) fn success(&self, message: &str) {
        if !self.stdout.is_quiet() {
            self.line(Role::Success, message);
        }
    }

    /// Prints an informational line to standard output unless quiet.
    pub(crate) fn info(&self, message: &str) {
        if !self.stdout.is_quiet() {
            self.line(Role::Info, message);
        }
    }

    /// Prints `message` to standard output with `role`, even when quiet.
    pub(crate) fn line(&self, role: Role, message: &str) {
        println!("{}", self.paint(message, role));
    }

    /// Prints a `Warning:` line to standard error.
    pub(crate) fn warn(&self, message: &str) {
        let label = self.stderr.theme.paint("Warning:", Role::Warn);
        let text = self.stderr.theme.paint(message, Role::Warn);

        eprintln!("{label} {text}");
    }

    /// Prints an `Error:` line to standard error.
    pub(crate) fn error(&self, message: &str) {
        let label = self.stderr.theme.paint("Error:", Role::Error);
        let text = self.stderr.theme.paint(message, Role::ErrorText);

        eprintln!("{label} {text}");
    }

    /// Paints `text` with the standard output theme.
    #[must_use]
    pub(crate) fn paint<'text>(&self, text: &'text str, role: Role) -> Styled<'text> {
        self.stdout.theme.paint(text, role)
    }

    /// Returns the standard output style for `role`.
    #[must_use]
    pub(crate) fn style(&self, role: Role) -> Style {
        self.stdout.theme.style(role)
    }

    /// Prints `table` to standard output.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub(crate) fn table(&self, table: &Table) -> io::Result<()> {
        let mut screen = Ansi::new(std::io::stdout(), self.stdout.size);

        table.render(&mut screen, self.stdout.theme)
    }

    /// Starts a live step display on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub(crate) fn steps(&self) -> io::Result<Steps> {
        Steps::stderr_with(self.stderr)
    }

    /// Starts a spinner on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    pub(crate) fn spinner(&self, message: &str) -> io::Result<Spinner> {
        Spinner::stderr_with(message, self.stderr)
    }
}

#[cfg(test)]
mod tests {
    use tui::glyphs::Glyphs;
    use tui::output::Mode;
    use tui::size::Size;
    use tui::style::Theme;

    use super::*;

    fn console(theme: Theme) -> Console {
        let output = Output {
            mode: Mode::Plain,
            theme,
            glyphs: Glyphs::plain(),
            size: Size::new(80, 25),
            verbose: false,
        };

        Console {
            stdout: output,
            stderr: output,
        }
    }

    #[test]
    fn plain_console_paints_without_escapes() {
        // ARRANGE
        let console = console(Theme::plain());

        // ACT
        let painted = console.paint("ok", Role::Success).to_string();

        // ASSERT
        assert_eq!(painted, "ok");
    }

    #[test]
    fn colored_console_paints_with_escapes() {
        // ARRANGE
        let console = console(Theme::colored());

        // ACT
        let painted = console.paint("ok", Role::Success).to_string();

        // ASSERT
        assert!(painted.contains('\u{1b}'), "painted: {painted:?}");
    }
}
