//! Live progress adapters for pipeline events.

use tui::live::StepsHandle;
use tui::live::tracker::Tracked as Progress;
use tui::progress::sink::Tracker;
use wizard::artifact::Artifact;
use wizard::progress::{Event, NodeState, Reporter};

/// Steps-backed reporter that renders pipeline events as one live line.
pub(super) struct Tracked {
    progress: Progress,
}

impl Tracked {
    pub(super) fn new(steps: StepsHandle, verbose: bool) -> Self {
        Self {
            progress: Progress::new(steps, verbose),
        }
    }

    fn sized(&self, artifacts: usize, bytes: Option<u64>) {
        self.progress.sized(building(artifacts), bytes);
    }
}

impl Reporter for Tracked {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { label } => self.progress.start(label),
            Event::Completed { label } => self.progress.complete(label),
            Event::Sized { artifacts, bytes } => self.sized(artifacts, bytes),
            Event::ArtifactBytes { artifact, current } => {
                let index = artifact.to_index().saturating_add(1);
                self.progress.bytes(Some(index), current, current);
            }
            Event::Node { node, state } => {
                self.progress
                    .note_state(node, state.title(), state == NodeState::Failed);
            }
            Event::Note {
                node,
                message,
                detail,
            } => self.progress.note_detail(node, message, detail),
        }
    }
}

/// Tracker that reports written artifact bytes to the CLI reporter.
pub(super) struct ArtifactTracker<'a> {
    artifact: Artifact,
    reporter: &'a dyn Reporter,
}

impl<'a> ArtifactTracker<'a> {
    pub(super) const fn new(artifact: Artifact, reporter: &'a dyn Reporter) -> Self {
        Self { artifact, reporter }
    }
}

impl Tracker for ArtifactTracker<'_> {
    fn advance(&self, current: u64) {
        self.reporter.report(Event::ArtifactBytes {
            artifact: self.artifact,
            current,
        });
    }
}

fn building(artifacts: usize) -> String {
    if artifacts == 1 {
        "Building 1 artifact".to_owned()
    } else {
        format!("Building {artifacts} artifacts")
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::io::Write as _;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use tui::live::{Options, Steps};
    use tui::progress::sink::Sink;
    use tui::screen::Screen;
    use tui::size::Size;
    use tui::style::Theme;
    use tui::text::Span;

    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<String>>);

    impl Reporter for Recorded {
        fn report(&self, event: Event<'_>) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push(format!("{event:?}"));
        }
    }

    struct RecordingScreen {
        tx: SyncSender<String>,
        size: Size,
    }

    impl RecordingScreen {
        fn new(size: Size) -> (Self, Receiver<String>) {
            let (tx, rx) = sync_channel(1024);

            (Self { tx, size }, rx)
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
            let _send_result = self.tx.try_send(span.text().to_owned());

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

    fn tracked(verbose: bool) -> (Tracked, Steps, Receiver<String>) {
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let steps = Steps::new(screen, Options::new(true, Theme::plain())).expect("steps");
        let tracked = Tracked::new(steps.handle(), verbose);

        (tracked, steps, rx)
    }

    fn output(rx: &Receiver<String>, steps: Steps) -> String {
        steps.finish().expect("finish");

        rx.iter().collect()
    }

    #[test]
    fn maps_stages_and_notes_to_the_live_display() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.report(Event::Started {
            label: "Resolving sources",
        });
        tracked.report(Event::Completed {
            label: "Resolved rpi4 (amd64)",
        });
        tracked.report(Event::Sized {
            artifacts: 1,
            bytes: Some(100),
        });
        tracked.report(Event::ArtifactBytes {
            artifact: Artifact::Iso,
            current: 100,
        });
        tracked.report(Event::Note {
            node: Some("kernel"),
            message: "Downloading layers",
            detail: false,
        });
        tracked.report(Event::Note {
            node: Some("kernel"),
            message: "sha256:abc 1024 bytes",
            detail: true,
        });
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("Resolving sources"), "output: {text}");
        assert!(text.contains("Resolved rpi4 (amd64)"), "output: {text}");
        assert!(text.contains("Building 1 artifact"), "output: {text}");
        assert!(
            text.contains("kernel: Downloading layers"),
            "output: {text}"
        );
        assert!(
            !text.contains("sha256:abc"),
            "detail notes must be verbose-only: {text}"
        );
    }

    #[test]
    fn aggregates_artifact_bytes_by_one_based_index() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.report(Event::Sized {
            artifacts: 2,
            bytes: Some(2048),
        });
        tracked.report(Event::ArtifactBytes {
            artifact: Artifact::Iso,
            current: 1024,
        });
        tracked.report(Event::ArtifactBytes {
            artifact: Artifact::Raw,
            current: 512,
        });
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("1.5 KiB / 2 KiB"), "output: {text}");
    }

    #[test]
    fn unknown_artifact_sizes_fall_back_to_a_spinner() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.report(Event::Sized {
            artifacts: 1,
            bytes: None,
        });
        tracked.report(Event::ArtifactBytes {
            artifact: Artifact::Iso,
            current: 4096,
        });
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("Building 1 artifact"), "output: {text}");
        assert!(
            !text.contains("0%"),
            "unknown sizes must not render a bar: {text}"
        );
    }

    #[test]
    fn verbose_mode_shows_node_states_and_detail() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(true);

        // ACT
        tracked.report(Event::Sized {
            artifacts: 1,
            bytes: Some(10),
        });
        tracked.report(Event::Node {
            node: "uki",
            state: NodeState::Started,
        });
        tracked.report(Event::Node {
            node: "uki",
            state: NodeState::Completed,
        });
        tracked.report(Event::Note {
            node: Some("uki"),
            message: "sha256:abc 1024 bytes",
            detail: true,
        });
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("uki: started"), "output: {text}");
        assert!(text.contains("uki: done"), "output: {text}");
        assert!(
            text.contains("uki: sha256:abc 1024 bytes"),
            "output: {text}"
        );
    }

    #[test]
    fn failed_nodes_are_reported_without_verbose() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.report(Event::Node {
            node: "iso",
            state: NodeState::Failed,
        });
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("iso: failed"), "output: {text}");
    }

    #[test]
    fn artifact_tracker_reports_cumulative_bytes_per_write() {
        // ARRANGE
        let recorded = Recorded::default();
        let file = tempfile::tempfile().expect("temp file");
        let mut sink = Sink::new(file, ArtifactTracker::new(Artifact::Uki, &recorded), 0);

        // ACT
        sink.write_all(b"hello").expect("write");
        sink.write_all(b" world").expect("write");

        // ASSERT
        let events = recorded.0.lock().expect("unpoisoned");
        assert_eq!(events.len(), 2, "events: {events:?}");
        assert!(events[0].contains("Uki"), "events: {events:?}");
        assert!(events[0].contains("current: 5"), "events: {events:?}");
        assert!(events[1].contains("current: 11"), "events: {events:?}");
    }
}
