//! Live progress display for the CLI.

use std::error::Error;
use std::io;

use anyhow::{Context as _, Result};
use koci::progress::report::Plain;
use koci::progress::{Event, Noop, Reporter};
use koci::ui::Terminal;
use tui::console::Console;
use tui::live::session::Session;
use tui::live::steps::Handle;
use tui::output::{Mode, Output};

// Reporter selected by the resolved output mode.
enum Live {
    Quiet(Noop),
    Plain(Plain),
    Rich(Terminal),
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
    let output = console.stderr();
    let progress = Session::new(output, starts_steps(output), reporter)?;
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

fn starts_steps(output: Output) -> bool {
    output.is_rich()
}

fn reporter(output: Output, handle: Option<Handle>) -> io::Result<Live> {
    match output.mode {
        Mode::Quiet => Ok(Live::Quiet(Noop)),
        Mode::Plain => Ok(Live::Plain(Plain {
            verbose: output.verbose,
        })),
        Mode::Rich => {
            let handle =
                handle.ok_or_else(|| io::Error::other("rich output did not start steps"))?;

            Ok(Live::Rich(Terminal::new(handle, output.verbose)))
        }
    }
}

#[cfg(test)]
mod tests {
    use tui::glyphs::Glyphs;
    use tui::output::Output;
    use tui::size::Size;
    use tui::style::Theme;

    use super::*;

    fn output(mode: Mode) -> Output {
        Output {
            mode,
            theme: Theme::plain(),
            glyphs: Glyphs::plain(),
            size: Size::new(80, 24),
            verbose: false,
        }
    }

    #[test]
    fn starts_steps_follows_the_rich_mode() {
        // ARRANGE
        let modes = [Mode::Quiet, Mode::Plain, Mode::Rich];

        // ACT
        let started = modes.map(|mode| starts_steps(output(mode)));

        // ASSERT
        assert_eq!(started, [false, false, true]);
    }

    #[test]
    fn reporter_requires_a_handle_only_when_steps_start() {
        // ARRANGE
        let quiet = output(Mode::Quiet);
        let plain = output(Mode::Plain);
        let rich = output(Mode::Rich);

        // ACT
        let quiet_live = reporter(quiet, None);
        let plain_live = reporter(plain, None);
        let rich_live = reporter(rich, None);

        // ASSERT
        assert!(matches!(quiet_live, Ok(Live::Quiet(_))));
        assert!(matches!(plain_live, Ok(Live::Plain(_))));
        assert!(rich_live.is_err());
    }
}
