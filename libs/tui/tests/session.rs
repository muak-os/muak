#![cfg(feature = "live")]

//! Integration tests for live progress sessions.

pub mod common;

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::Receiver;

    use tui::glyphs::Glyphs;
    use tui::live::Options;
    use tui::live::session::Session;
    use tui::live::steps::Steps;
    use tui::output::{Mode, Output};
    use tui::size::Size;
    use tui::style::Theme;

    use crate::common::recording::{Recorder, collect};

    fn make_output(mode: Mode) -> Output {
        Output {
            mode,
            theme: Theme::plain(),
            glyphs: Glyphs::plain(),
            size: Size::new(80, 24),
            verbose: false,
        }
    }

    fn steps(output: Output) -> (Steps, Receiver<String>) {
        let (screen, rx) = Recorder::new(Size::new(80, 24));

        (
            Steps::new(screen, Options::from_output(output)).expect("steps"),
            rx,
        )
    }

    #[test]
    fn no_steps_session_passes_none() {
        // ARRANGE
        let output = make_output(Mode::Plain);

        // ACT
        let session =
            Session::new(output, false, |_output, handle| Ok(handle.is_some())).expect("session");

        // ASSERT
        assert!(!*session.reporter());
        session.finish().expect("finish");
    }

    #[test]
    fn with_steps_session_passes_handle() {
        // ARRANGE
        let output = make_output(Mode::Plain);
        let (steps, _rx) = steps(output);

        // ACT
        let session =
            Session::with_steps(output, Some(steps), |_output, handle| Ok(handle.is_some()))
                .expect("session");

        // ASSERT
        assert!(*session.reporter());
        session.finish().expect("finish");
    }

    #[test]
    fn reporter_error_tears_down_the_renderer() {
        // ARRANGE
        let output = make_output(Mode::Plain);
        let (steps, rx) = steps(output);

        // ACT
        let result =
            Session::with_steps(output, Some(steps), |_output, _handle| -> io::Result<()> {
                Err(io::Error::other("reporter failed"))
            });
        let rendered = collect(&rx);

        // ASSERT
        assert!(result.is_err());
        assert!(rendered.is_empty(), "rendered: {rendered:?}");
    }

    #[test]
    fn finish_with_steps_stops_the_renderer() {
        // ARRANGE
        let output = make_output(Mode::Plain);
        let (steps, rx) = steps(output);
        steps.start("orphan");
        let session =
            Session::with_steps(output, Some(steps), |_output, _handle| Ok(())).expect("session");

        // ACT
        session.finish().expect("finish");
        let rendered = collect(&rx);

        // ASSERT
        assert!(rendered.contains("[ok] orphan"), "rendered: {rendered:?}");
    }

    #[test]
    fn fail_without_steps_succeeds() {
        // ARRANGE
        let output = make_output(Mode::Plain);
        let session = Session::new(output, false, |_output, _handle| Ok(())).expect("session");

        // ACT
        let result = session.fail("broken");

        // ASSERT
        result.expect("fail");
    }

    #[test]
    fn fail_with_steps_renders_the_failure() {
        // ARRANGE
        let output = make_output(Mode::Plain);
        let (steps, rx) = steps(output);
        let session =
            Session::with_steps(output, Some(steps), |_output, _handle| Ok(())).expect("session");

        // ACT
        session.fail("broken").expect("fail");
        let rendered = collect(&rx);

        // ASSERT
        assert!(rendered.contains("broken"), "rendered: {rendered:?}");
    }
}
