#![cfg(feature = "live")]

//! Integration tests for live widgets.

use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use tui::live::Options;
use tui::screen::Screen;
use tui::size::Size;
use tui::style::Theme;
use tui::text::Span;

struct RecordingScreen {
    tx: SyncSender<String>,
    size: Size,
}

impl RecordingScreen {
    fn new(size: Size) -> (Self, Receiver<String>) {
        let (tx, rx) = sync_channel(1024);

        (Self { tx, size }, rx)
    }

    fn send(&self, text: impl Into<String>) {
        let _send_result = self.tx.try_send(text.into());
    }
}

impl Screen for RecordingScreen {
    fn size(&self) -> Size {
        self.size
    }

    fn move_to(&mut self, _col: u16, _row: u16) -> io::Result<()> {
        Ok(())
    }

    fn move_to_column(&mut self, _col: u16) -> io::Result<()> {
        Ok(())
    }

    fn clear_until_newline(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_span(&mut self, span: &Span<'_>) -> io::Result<()> {
        self.send(span.text());

        Ok(())
    }

    fn set_scroll_region(&mut self, _top: u16, _bottom: u16) -> io::Result<()> {
        Ok(())
    }

    fn reset_scroll_region(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn options(tty: bool) -> Options {
    Options {
        tty,
        theme: Theme::plain(),
    }
}

fn collect(rx: &Receiver<String>) -> String {
    rx.iter().collect()
}

#[cfg(test)]
mod tests {
    use tui::live::{Spinner, Steps};

    use super::*;

    #[test]
    fn spinner_renders_updates_and_success() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(60, 10));
        let spinner = Spinner::start(screen, "working", options(true)).unwrap();

        // ACT
        spinner.update("still working");
        spinner.success("done").unwrap();
        let output = collect(&rx);

        // ASSERT
        assert!(output.contains("still working"), "output: {output}");
        assert!(output.contains("done"), "output: {output}");
        assert!(output.contains('\u{2713}'), "output: {output}");
    }

    #[test]
    fn spinner_renders_failure() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(60, 10));
        let spinner = Spinner::start(screen, "working", options(true)).unwrap();

        // ACT
        spinner.fail("broken").unwrap();
        let output = collect(&rx);

        // ASSERT
        assert!(output.contains("broken"), "output: {output}");
        assert!(output.contains('\u{2717}'), "output: {output}");
    }

    #[test]
    fn spinner_plain_output_ignores_later_updates() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(60, 10));
        let spinner = Spinner::start(screen, "waiting", options(false)).unwrap();

        // ACT
        spinner.update("ignored");
        spinner.success("ready").unwrap();
        let output = collect(&rx);

        // ASSERT
        assert!(output.contains("waiting"), "output: {output}");
        assert!(output.contains("ready"), "output: {output}");
        assert!(!output.contains("ignored"), "output: {output}");
    }

    #[test]
    fn steps_render_ordered_phases() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let steps = Steps::new(screen, options(true)).unwrap();

        // ACT
        steps.start("first");
        steps.start("second");
        steps.start_progress("download", 10);
        steps.progress(5);
        steps.set_label("downloading");
        steps.complete("downloaded");
        steps.start("third");
        steps.fail("broken");
        steps.finish().unwrap();
        let output = collect(&rx);

        // ASSERT
        assert!(output.contains("first"), "output: {output}");
        assert!(output.contains("second"), "output: {output}");
        assert!(output.contains("downloading"), "output: {output}");
        assert!(output.contains("50%"), "output: {output}");
        assert!(output.contains("downloaded"), "output: {output}");
        assert!(output.contains("third"), "output: {output}");
        assert!(output.contains("broken"), "output: {output}");
        assert!(output.contains('\u{2713}'), "output: {output}");
        assert!(output.contains('\u{2717}'), "output: {output}");
    }

    #[test]
    fn steps_plain_output_prints_only_phases() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let steps = Steps::new(screen, options(false)).unwrap();

        // ACT
        steps.start("first");
        steps.set_label("ignored");
        steps.progress(5);
        steps.complete("done");
        steps.fail("broken");
        steps.finish().unwrap();
        let output = collect(&rx);

        // ASSERT
        assert_eq!(output.lines().count(), 3, "output: {output}");
        assert!(output.contains("first"), "output: {output}");
        assert!(output.contains("done"), "output: {output}");
        assert!(output.contains("broken"), "output: {output}");
        assert!(!output.contains("ignored"), "output: {output}");
    }

    #[test]
    fn dropping_steps_freezes_the_active_step() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let steps = Steps::new(screen, options(true)).unwrap();

        // ACT
        steps.start("orphan");
        drop(steps);
        let output = collect(&rx);

        // ASSERT
        assert!(output.contains("orphan"), "output: {output}");
        assert!(output.contains('\u{2713}'), "output: {output}");
    }
}
