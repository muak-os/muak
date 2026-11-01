//! Live progress display for the CLI.

use std::error::Error;
use std::io;

use anyhow::{Context as _, Result};
use koci::progress::report::Plain;
use koci::progress::{Event, Noop, Reporter};
use koci::ui::terminal::Terminal;
use tui::console::Console;
use tui::live::Steps;
use tui::output::{Mode, Output};

// Reporter selected by the resolved output mode.
enum Live {
    Quiet(Noop),
    Plain(Plain),
    Rich(Terminal),
}

// Live progress display of one CLI operation.
struct Progress {
    steps: Option<Steps>,
    live: Live,
}

impl Reporter for Live {
    fn report(&self, event: Event<'_>) {
        match *self {
            Self::Quiet(ref reporter) => reporter.report(event),
            Self::Plain(ref reporter) => reporter.report(event),
            Self::Rich(ref reporter) => reporter.report(event),
        }
    }
}

impl Progress {
    fn new(output: Output) -> io::Result<Self> {
        match output.mode {
            Mode::Quiet => Ok(Self {
                steps: None,
                live: Live::Quiet(Noop),
            }),
            Mode::Plain => Ok(Self {
                steps: None,
                live: Live::Plain(Plain {
                    verbose: output.verbose,
                }),
            }),
            Mode::Rich => {
                let steps = Steps::stderr_with(output)?;
                let live = Live::Rich(Terminal::new(steps.handle(), output.verbose));

                Ok(Self {
                    steps: Some(steps),
                    live,
                })
            }
        }
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

/// Runs `operation` behind a live progress display.
///
/// # Errors
///
/// Returns the operation error, annotated with the failed operation.
pub(crate) fn tracked<T, E>(
    console: &Console,
    operation: &str,
    run: impl FnOnce(&dyn Reporter) -> Result<T, E>,
) -> Result<T>
where
    E: Error + Send + Sync + 'static,
{
    let progress = Progress::new(console.stderr())?;
    match run(progress.reporter()) {
        Ok(value) => {
            progress.finish()?;
            Ok(value)
        }
        Err(error) => {
            progress.fail(format!("Failed to {operation}: {error}"))?;
            Err(error).context(format!("Failed to {operation}"))
        }
    }
}
