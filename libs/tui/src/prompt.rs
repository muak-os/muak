//! Interactive terminal prompts.

use core::fmt;
use std::io::{self, BufRead, Write};

use crate::style::{Role, Stream, Theme};

/// Interactive prompt over an explicit reader and writer.
pub struct Prompter<R, W> {
    reader: R,
    writer: W,
    theme: Theme,
}

impl<R: BufRead, W: Write> Prompter<R, W> {
    /// Creates a prompter over `reader` and `writer`.
    pub fn new(reader: R, writer: W, theme: Theme) -> Self {
        Self {
            reader,
            writer,
            theme,
        }
    }

    /// Asks for a yes/no confirmation.
    ///
    /// # Errors
    ///
    /// Returns an error when reading or writing fails.
    pub fn confirm(&mut self, message: &str) -> io::Result<bool> {
        let input = self.ask(format_args!("{message} {} ", self.theme.muted("[y/N]")))?;

        Ok(matches!(input.to_lowercase().as_str(), "y" | "yes"))
    }

    /// Asks the user to type an exact phrase.
    ///
    /// # Errors
    ///
    /// Returns an error when reading or writing fails.
    pub fn confirm_phrase(&mut self, message: &str, phrase: &str) -> io::Result<bool> {
        let input = self.ask(format_args!(
            "{message} Type {} to confirm: ",
            self.theme.paint(phrase, Role::Label)
        ))?;

        Ok(input == phrase)
    }

    /// Returns the underlying reader and writer.
    #[must_use]
    pub fn into_inner(self) -> (R, W) {
        (self.reader, self.writer)
    }

    fn ask(&mut self, prompt: fmt::Arguments<'_>) -> io::Result<String> {
        self.writer.write_fmt(prompt)?;
        self.writer.flush()?;

        let mut input = String::new();
        self.reader.read_line(&mut input)?;

        Ok(input.trim().to_owned())
    }
}

impl Prompter<io::StdinLock<'static>, io::StdoutLock<'static>> {
    /// Creates a prompter on standard input and output when stdout is a terminal.
    #[must_use]
    pub fn stdio() -> Option<Self> {
        if !Stream::Stdout.is_terminal() {
            return None;
        }

        Some(Self::new(
            io::stdin().lock(),
            io::stdout().lock(),
            Theme::detect(Stream::Stdout),
        ))
    }
}
