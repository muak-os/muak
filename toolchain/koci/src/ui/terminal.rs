//! Terminal progress reporter rendered with `tui` steps.

use std::sync::Mutex;

use tui::live::StepsHandle;
use tui::units;

use crate::progress::{Event, Phase, Reporter, Unit};
use crate::ui::bytes::ByteState;

/// Terminal progress reporter rendered with `tui` steps.
pub struct Terminal {
    steps: StepsHandle,
    verbose: bool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    bytes: ByteState,
}

impl Terminal {
    /// Creates a terminal reporter over `steps`.
    #[must_use]
    pub fn new(steps: StepsHandle, verbose: bool) -> Self {
        Self {
            steps,
            verbose,
            state: Mutex::new(State::default()),
        }
    }

    fn start_phase(&self, label: &str, total: u64, unit: Unit) {
        if let Ok(mut state) = self.state.lock() {
            state.bytes = ByteState::default();
        }
        match unit {
            Unit::Bytes => self.steps.start_bytes(label, total),
            Unit::Items => self.steps.start_items(label, total),
        }
    }

    fn progress(&self, phase: Phase, current: u64, total: u64) {
        self.steps
            .set_label(format!("{} {current}/{total}", phase.title()));
        self.steps.progress(current);
    }

    fn byte_progress(&self, index: Option<usize>, current: u64, total: u64) {
        if let Some(index) = index {
            let current = match self.state.lock() {
                Ok(mut state) => state.bytes.sample(index, current, total),
                Err(_poisoned) => return,
            };
            self.steps.progress(current);
        } else {
            self.steps.progress(current.min(total));
        }
    }

    fn set_size(&self, index: usize, size: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.bytes.set_size(index, size);
        }
    }

    fn detail(&self, message: &str) {
        if self.verbose {
            self.steps.note(message);
        }
    }

    fn item_done(&self, index: usize, size: u64, digest: &str) {
        let current = match self.state.lock() {
            Ok(mut state) => state.bytes.finish(index),
            Err(_poisoned) => return,
        };
        self.steps.progress(current);
        if self.verbose {
            self.steps.note(format!("{digest}  {}", units::bytes(size)));
        }
    }
}

impl Reporter for Terminal {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { operation, subject } => {
                self.steps.start(format!("{} {subject}", operation.title()));
            }
            Event::Resolved { layers, .. } => {
                self.steps.complete(format!("Resolved {layers} layers"));
            }
            Event::Collected { manifests, blobs } => {
                self.steps
                    .complete(format!("Collected {manifests} manifests, {blobs} blobs"));
            }
            Event::Packed { entries, bytes } => {
                self.steps.complete(format!(
                    "Packed {entries} entries ({})",
                    units::bytes(bytes)
                ));
            }
            Event::IndexFetched { platforms } => {
                self.steps
                    .complete(format!("Fetched index ({platforms} platforms)"));
            }
            Event::PhaseStarted {
                label, total, unit, ..
            } => self.start_phase(label, total, unit),
            Event::PhaseCompleted { label, .. } => {
                self.steps.complete(label);
            }
            Event::Progress {
                phase,
                current,
                total,
            } => self.progress(phase, current, total),
            Event::Bytes {
                index,
                current,
                total,
                ..
            } => self.byte_progress(index, current, total),
            Event::ItemQueued { index, size, .. } => self.set_size(index, size),
            Event::ItemSkipped { digest, .. } => {
                self.detail(&format!("Skipped existing {digest}"));
            }
            Event::ItemDone {
                index,
                size,
                digest,
                ..
            } => self.item_done(index, size, digest),
            Event::ManifestPushed { reference } => {
                self.detail(&format!("Pushed manifest {reference}"));
            }
            Event::Verified { digest } => {
                self.steps
                    .complete(format!("Verified destination digest {digest}"));
            }
            Event::Note { message } => self.detail(message),
            Event::Done { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use tui::live::{Options, Steps};
    use tui::screen::Screen;
    use tui::size::Size;
    use tui::style::Theme;
    use tui::text::Span;

    use super::*;
    use crate::progress::{Operation, Unit};

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

    fn reporter(verbose: bool) -> (Terminal, Steps, Receiver<String>) {
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let steps = Steps::new(screen, Options::new(true, Theme::plain())).expect("steps");
        let reporter = Terminal::new(steps.handle(), verbose);

        (reporter, steps, rx)
    }

    #[test]
    fn maps_events_to_ordered_phases() {
        // ARRANGE
        let (reporter, steps, rx) = reporter(false);

        // ACT
        reporter.report(Event::Started {
            operation: Operation::Pull,
            subject: "repo:test linux/amd64",
        });
        reporter.report(Event::Resolved {
            layers: 2,
            digest: "sha256:abc",
        });
        reporter.report(Event::ItemQueued {
            phase: Phase::Download,
            index: 1,
            total: 2,
            digest: "sha256:aaa",
            size: 1024,
        });
        reporter.report(Event::ItemQueued {
            phase: Phase::Download,
            index: 2,
            total: 2,
            digest: "sha256:bbb",
            size: 2048,
        });
        reporter.report(Event::PhaseStarted {
            phase: Phase::Download,
            label: "Downloading layers",
            total: 3072,
            unit: Unit::Bytes,
        });
        reporter.report(Event::Bytes {
            phase: Phase::Download,
            index: Some(1),
            current: 1024,
            total: 1024,
        });
        reporter.report(Event::ItemDone {
            phase: Phase::Download,
            index: 1,
            size: 1024,
            digest: "sha256:aaa",
        });
        reporter.report(Event::PhaseCompleted {
            phase: Phase::Download,
            label: "Downloaded 2/2 layers",
        });
        reporter.report(Event::PhaseStarted {
            phase: Phase::Extract,
            label: "Extracting layers",
            total: 2,
            unit: Unit::Items,
        });
        reporter.report(Event::Progress {
            phase: Phase::Extract,
            current: 2,
            total: 2,
        });
        reporter.report(Event::PhaseCompleted {
            phase: Phase::Extract,
            label: "Extracted 2/2 layers",
        });
        steps.finish().expect("finish");
        let output: String = rx.iter().collect();

        // ASSERT
        assert!(output.contains("koci pull repo:test"), "output: {output}");
        assert!(output.contains("Resolved 2 layers"), "output: {output}");
        assert!(output.contains("Downloading layers"), "output: {output}");
        assert!(output.contains("Downloaded 2/2 layers"), "output: {output}");
        assert!(output.contains("Extracted 2/2 layers"), "output: {output}");
    }

    #[test]
    fn verbose_reporter_notes_layer_details() {
        // ARRANGE
        let (reporter, steps, rx) = reporter(true);
        let digest = "sha256:3f2a9c1d8e7f0000000000000000000000000000000000000000000000000000";

        // ACT
        reporter.report(Event::Started {
            operation: Operation::Copy,
            subject: "source -> destination",
        });
        reporter.report(Event::Collected {
            manifests: 1,
            blobs: 1,
        });
        reporter.report(Event::PhaseStarted {
            phase: Phase::CopyBlobs,
            label: "Copying blobs",
            total: 1024,
            unit: Unit::Bytes,
        });
        reporter.report(Event::ItemDone {
            phase: Phase::CopyBlobs,
            index: 1,
            size: 1024,
            digest,
        });
        steps.finish().expect("finish");
        let output: String = rx.iter().collect();

        // ASSERT
        assert!(output.contains(digest), "output: {output}");
        assert!(output.contains("1 KiB"), "output: {output}");
    }
}
