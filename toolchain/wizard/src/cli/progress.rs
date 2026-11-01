//! Live progress display for the CLI.

use std::io;

use anyhow::Result;
use tui::console::Console;
use tui::live::Steps;
use tui::output::{Mode, Output};
use wizard::progress::{Event, Noop, Reporter};

use super::tracked::Tracked;

// Reporter selected by the resolved output mode.
enum Live {
    Quiet(Noop),
    Steps(Tracked),
}

// Live progress display of one CLI run.
struct Progress {
    steps: Option<Steps>,
    live: Live,
}

impl Reporter for Live {
    fn report(&self, event: Event<'_>) {
        match *self {
            Self::Quiet(ref reporter) => reporter.report(event),
            Self::Steps(ref reporter) => reporter.report(event),
        }
    }
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

/// Runs `build` behind a live progress display.
///
/// # Errors
///
/// Returns the build error after marking the active step as failed.
pub(super) fn tracked<T>(
    console: &Console,
    build: impl FnOnce(&dyn Reporter) -> Result<T>,
) -> Result<T> {
    let progress = Progress::new(console.stderr())?;
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
