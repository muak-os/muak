//! Live progress display for the CLI.

use std::io;

use anyhow::Result;
use tui::console::Console;
use tui::live::StepsHandle;
use tui::live::session::Session;
use tui::output::{Mode, Output};
use wizard::progress::{Event, Noop, Reporter};

use super::tracked::Tracked;

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

/// Runs `build` behind a live progress display.
///
/// # Errors
///
/// Returns the build error after marking the active step as failed.
pub(super) fn tracked<T>(
    console: &Console,
    build: impl FnOnce(&dyn Reporter) -> Result<T>,
) -> Result<T> {
    let output = console.stderr();
    let progress = Session::new(output, starts_steps(output), reporter)?;

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

fn starts_steps(output: Output) -> bool {
    !output.is_quiet()
}

fn reporter(output: Output, handle: Option<StepsHandle>) -> io::Result<Live> {
    match output.mode {
        Mode::Quiet => Ok(Live::Quiet(Noop)),
        Mode::Plain | Mode::Rich => {
            let handle =
                handle.ok_or_else(|| io::Error::other("non-quiet output did not start steps"))?;

            Ok(Live::Steps(Tracked::new(handle, output.verbose)))
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
    fn starts_steps_follows_the_non_quiet_modes() {
        // ARRANGE
        let modes = [Mode::Quiet, Mode::Plain, Mode::Rich];

        // ACT
        let started = modes.map(|mode| starts_steps(output(mode)));

        // ASSERT
        assert_eq!(started, [false, true, true]);
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
        assert!(plain_live.is_err());
        assert!(rich_live.is_err());
    }
}
