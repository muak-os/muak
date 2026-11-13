//! Terminal progress reporter rendered with `tui` steps.

use tui::live::steps::Handle;
use tui::live::tracker::Tracked;
use tui::units;

use crate::progress::{Event, Phase, Reporter, Unit};

/// Terminal progress reporter rendered with `tui` steps.
pub struct Terminal {
    tracked: Tracked,
}

impl Terminal {
    /// Creates a terminal reporter over `steps`.
    #[must_use = "dropping it discards the reporter state"]
    pub fn new(steps: Handle, verbose: bool) -> Self {
        Self {
            tracked: Tracked::new(steps, verbose),
        }
    }

    fn start_phase(&self, label: &str, total: u64, unit: Unit) {
        match unit {
            Unit::Bytes => self.tracked.start_bytes(label, total),
            Unit::Items => self.tracked.start_items(label, total),
        }
    }

    fn progress(&self, phase: Phase, current: u64, total: u64) {
        self.tracked
            .set_label(format!("{} {current}/{total}", phase.title()));
        self.tracked.progress(current);
    }

    fn item_done(&self, index: usize, size: u64, digest: &str) {
        self.tracked.item_done(index);
        self.tracked
            .note_if_verbose(format!("{digest}  {}", units::bytes(size)));
    }
}

impl Reporter for Terminal {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { operation, subject } => {
                self.tracked
                    .start(format!("{} {subject}", operation.title()));
            }
            Event::Resolved { layers, .. } => {
                self.tracked.complete(format!("Resolved {layers} layers"));
            }
            Event::Collected { manifests, blobs } => {
                self.tracked
                    .complete(format!("Collected {manifests} manifests, {blobs} blobs"));
            }
            Event::Packed { entries, bytes } => {
                self.tracked.complete(format!(
                    "Packed {entries} entries ({})",
                    units::bytes(bytes)
                ));
            }
            Event::IndexFetched { platforms } => {
                self.tracked
                    .complete(format!("Fetched index ({platforms} platforms)"));
            }
            Event::PhaseStarted {
                label, total, unit, ..
            } => self.start_phase(label, total, unit),
            Event::PhaseCompleted { label, .. } => {
                self.tracked.complete(label);
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
            } => self.tracked.bytes(index, current, total),
            Event::ItemQueued { index, size, .. } => self.tracked.set_size(index, size),
            Event::ItemSkipped { digest, .. } => {
                self.tracked
                    .note_if_verbose(format!("Skipped existing {digest}"));
            }
            Event::ItemDone {
                index,
                size,
                digest,
                ..
            } => self.item_done(index, size, digest),
            Event::ManifestPushed { reference } => {
                self.tracked
                    .note_if_verbose(format!("Pushed manifest {reference}"));
            }
            Event::Verified { digest } => {
                self.tracked
                    .complete(format!("Verified destination digest {digest}"));
            }
            Event::Note { message } => self.tracked.note_if_verbose(message),
            Event::Done { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use tui::live::Options;
    use tui::live::steps::Steps;
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
