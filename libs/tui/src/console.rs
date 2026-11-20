//! Console facade shared by the command-line interfaces.

use std::io::Write as _;

#[cfg(feature = "live")]
use crate::live::spinner::Spinner;
#[cfg(feature = "live")]
use crate::live::steps::Steps;
#[cfg(feature = "live")]
use crate::live::summary::{self, Summary};
use crate::output::{Flags, Output};
use crate::screen::ansi::Ansi;
use crate::style::{Role, Stream, Style, Styled};
use crate::table::Table;

/// Resolved standard output and standard error for one command-line run.
pub struct Console {
    stdout: Output,
    stderr: Output,
}

impl Console {
    /// Creates a console from already resolved `stdout` and `stderr`.
    #[must_use = "dropping it discards the chosen streams"]
    pub const fn new(stdout: Output, stderr: Output) -> Self {
        Self { stdout, stderr }
    }

    /// Detects both streams from the global output `flags`.
    #[must_use = "dropping it discards the detected terminal settings"]
    pub fn detect(flags: Flags) -> Self {
        Self {
            stdout: Output::detect(Stream::Stdout, flags),
            stderr: Output::detect(Stream::Stderr, flags),
        }
    }

    /// Returns the resolved standard output.
    #[must_use = "callers need the stream to write output"]
    pub const fn stdout(&self) -> Output {
        self.stdout
    }

    /// Returns the resolved standard error.
    #[must_use = "callers need the stream to write output"]
    pub const fn stderr(&self) -> Output {
        self.stderr
    }

    /// Prints a success line to standard output unless quiet.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn success(&self, message: &str) -> std::io::Result<()> {
        if self.stdout.is_quiet() {
            return Ok(());
        }

        self.line(Role::Success, message)
    }

    /// Prints an informational line to standard output unless quiet.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn info(&self, message: &str) -> std::io::Result<()> {
        if self.stdout.is_quiet() {
            return Ok(());
        }

        self.line(Role::Info, message)
    }

    /// Prints `message` to standard output with `role`, even when quiet.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn line(&self, role: Role, message: &str) -> std::io::Result<()> {
        writeln!(std::io::stdout(), "{}", self.paint(message, role))
    }

    /// Writes `message` to standard output verbatim, without a trailing newline, even when quiet.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn write(&self, message: &str) -> std::io::Result<()> {
        std::io::stdout().write_all(message.as_bytes())
    }

    /// Formats a `Warning:` line for standard error.
    #[must_use = "callers need the styled text to print it"]
    pub fn warning(&self, message: &str) -> String {
        let label = self.stderr.theme.paint("Warning:", Role::Warn);
        let text = self.stderr.theme.paint(message, Role::Warn);

        format!("{label} {text}")
    }

    /// Prints a `Warning:` line to standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard error fails.
    pub fn warn(&self, message: &str) -> std::io::Result<()> {
        writeln!(std::io::stderr(), "{}", self.warning(message))
    }

    /// Formats an `Error:` line for standard error.
    #[must_use = "callers need the styled text to print it"]
    pub fn failure(&self, message: &str) -> String {
        let label = self.stderr.theme.paint("Error:", Role::Error);
        let text = self.stderr.theme.paint(message, Role::ErrorText);

        format!("{label} {text}")
    }

    /// Prints an `Error:` line to standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard error fails.
    pub fn error(&self, message: &str) -> std::io::Result<()> {
        writeln!(std::io::stderr(), "{}", self.failure(message))
    }

    /// Paints `text` with the standard output theme.
    #[must_use = "callers need the styled text to print it"]
    pub fn paint<'text>(&self, text: &'text str, role: Role) -> Styled<'text> {
        self.stdout.theme.paint(text, role)
    }

    /// Returns the standard output style for `role`.
    #[must_use = "callers need the style to paint text"]
    pub fn style(&self, role: Role) -> Style {
        self.stdout.theme.style(role)
    }

    /// Prints `table` to standard output, fitting it in rich mode.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn table(&self, table: &Table) -> std::io::Result<()> {
        let cols = self.stdout.is_rich().then_some(self.stdout.size.cols);

        self.write_table(table, cols)
    }

    /// Prints `table` to standard output at its natural width.
    ///
    /// # Errors
    ///
    /// Returns an error when writing to standard output fails.
    pub fn table_unfitted(&self, table: &Table) -> std::io::Result<()> {
        self.write_table(table, None)
    }

    /// Prints `summary` to standard output, warning when it cannot be written.
    #[cfg(feature = "live")]
    pub fn report(&self, summary: &Summary<'_>) {
        if let Err(error) = summary::print(summary, self.stdout) {
            drop(self.warn(&format!("Failed to print summary: {error}")));
        }
    }

    /// Starts a live step display on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    #[cfg(feature = "live")]
    pub fn steps(&self) -> std::io::Result<Steps> {
        Steps::stderr_with(self.stderr)
    }

    /// Starts a spinner on standard error.
    ///
    /// # Errors
    ///
    /// Returns an error when the renderer thread cannot start.
    #[cfg(feature = "live")]
    pub fn spinner(&self, message: &str) -> std::io::Result<Spinner> {
        Spinner::stderr_with(message, self.stderr)
    }

    fn write_table(&self, table: &Table, cols: Option<u16>) -> std::io::Result<()> {
        let mut screen = Ansi::new(std::io::stdout(), self.stdout.size);

        match cols {
            Some(cols) => table.render_fitted(&mut screen, self.stdout.theme, cols),
            None => table.render(&mut screen, self.stdout.theme),
        }
    }
}
