//! Console capabilities and live progress for the CLI.

use std::io;

use anyhow::Result;
use tui::live::Steps;
use tui::live::summary::{self, Summary};
use tui::output::{Flags, Mode, Output};
use tui::style::Stream;
use wizard::progress::{Event, Noop, Reporter};

use super::tracked::Tracked;

/// Resolved standard output and standard error for one CLI run.
pub(super) struct Console {
    stdout: Output,
    stderr: Output,
}

impl Console {
    /// Detects the capabilities of both streams from `flags`.
    pub(super) fn detect(flags: Flags) -> Self {
        Self {
            stdout: Output::detect(Stream::Stdout, flags),
            stderr: Output::detect(Stream::Stderr, flags),
        }
    }

    /// Runs `build` behind a live progress display.
    ///
    /// # Errors
    ///
    /// Returns the build error after marking the active step as failed.
    pub(super) fn tracked<T>(&self, build: impl FnOnce(&dyn Reporter) -> Result<T>) -> Result<T> {
        let progress = Progress::new(self.stderr)?;
        match build(progress.reporter()) {
            Ok(value) => {
                progress.finish()?;
                Ok(value)
            }
            Err(error) => {
                progress.fail(format!("Build failed: {error}"))?;
                Err(error)
            }
        }
    }

    /// Prints `summary` to standard output.
    pub(super) fn report(&self, summary: &Summary<'_>) {
        if let Err(error) = summary::print(summary, self.stdout) {
            self.warn(&format!("Failed to print summary: {error}"));
        }
    }

    /// Prints a warning to standard error.
    pub(super) fn warn(&self, message: &str) {
        eprintln!("{}", self.stderr.theme.warn(message));
    }
}

// Reporter selected by the resolved output mode.
enum Live {
    Quiet(Noop),
    Steps(Tracked),
}

impl Reporter for Live {
    fn report(&self, event: Event<'_>) {
        match *self {
            Self::Quiet(ref reporter) => reporter.report(event),
            Self::Steps(ref reporter) => reporter.report(event),
        }
    }
}

// Live progress display of one CLI run.
struct Progress {
    steps: Option<Steps>,
    live: Live,
}

impl Progress {
    fn new(output: Output) -> io::Result<Self> {
        if output.mode == Mode::Quiet {
            return Ok(Self {
                steps: None,
                live: Live::Quiet(Noop),
            });
        }

        let steps = Steps::stderr_with(output)?;
        let live = Live::Steps(Tracked::new(steps.handle(), output.verbose));

        Ok(Self {
            steps: Some(steps),
            live,
        })
    }

    fn reporter(&self) -> &dyn Reporter {
        &self.live
    }

    fn finish(self) -> io::Result<()> {
        match self.steps {
            Some(steps) => steps.finish(),
            None => Ok(()),
        }
    }

    fn fail(self, message: String) -> io::Result<()> {
        match self.steps {
            Some(steps) => {
                steps.fail(message);
                steps.finish()
            }
            None => Ok(()),
        }
    }
}
