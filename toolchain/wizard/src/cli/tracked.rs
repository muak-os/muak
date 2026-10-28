//! Steps-backed reporter for pipeline events.

use std::sync::{Mutex, PoisonError};

use tui::live::StepsHandle;
use wizard::artifact::Artifact;
use wizard::progress::{Event, NodeState, Reporter};

/// Steps-backed reporter that renders pipeline events as one live line.
pub(super) struct Tracked {
    steps: StepsHandle,
    verbose: bool,
    state: Mutex<State>,
}

impl Tracked {
    pub(super) fn new(steps: StepsHandle, verbose: bool) -> Self {
        Self {
            steps,
            verbose,
            state: Mutex::new(State::default()),
        }
    }

    fn node(&self, node: &str, state: NodeState) {
        if self.verbose || state == NodeState::Failed {
            self.steps.note(format!("{node}: {}", state.title()));
        }
    }

    fn note(&self, node: Option<&str>, message: &str, detail: bool) {
        if detail && !self.verbose {
            return;
        }

        match node {
            Some(node) => self.steps.note(format!("{node}: {message}")),
            None => self.steps.note(message.to_owned()),
        }
    }

    fn sized(&self, artifacts: usize, bytes: Option<u64>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reset();
        let label = building(artifacts);
        if let Some(bytes) = bytes {
            self.steps.start_bytes(label, bytes);
            return;
        }

        self.steps.start(label);
    }

    fn artifact_bytes(&self, artifact: Artifact, current: u64) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.set(artifact, current);
        let total = state.current;
        drop(state);
        self.steps.progress(total);
    }
}

impl Reporter for Tracked {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { label } => self.steps.start(label),
            Event::Completed { label } => self.steps.complete(label),
            Event::Sized { artifacts, bytes } => self.sized(artifacts, bytes),
            Event::ArtifactBytes { artifact, current } => self.artifact_bytes(artifact, current),
            Event::Node { node, state } => self.node(node, state),
            Event::Note {
                node,
                message,
                detail,
            } => self.note(node, message, detail),
        }
    }
}

fn building(artifacts: usize) -> String {
    if artifacts == 1 {
        "Building 1 artifact".to_owned()
    } else {
        format!("Building {artifacts} artifacts")
    }
}

// Bytes written so far to each artifact target.
#[derive(Default)]
struct State {
    artifacts: Vec<(Artifact, u64)>,
    current: u64,
}

impl State {
    fn reset(&mut self) {
        self.artifacts.clear();
        self.current = 0;
    }

    fn set(&mut self, artifact: Artifact, current: u64) {
        if let Some(entry) = self.artifacts.iter_mut().find(|entry| entry.0 == artifact) {
            self.current = self.current.saturating_sub(entry.1).saturating_add(current);
            entry.1 = current;
        } else {
            self.artifacts.push((artifact, current));
            self.current = self.current.saturating_add(current);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use tui::live::Options;
    use tui::live::Steps;
    use tui::screen::Screen;
    use tui::size::Size;
    use tui::style::Theme;
    use tui::text::Span;

    use super::*;

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
    fn artifact_bytes_aggregate_across_targets() {
        // ARRANGE
        let mut state = State::default();

        // ACT
        state.set(Artifact::Iso, 100);
        state.set(Artifact::Raw, 50);
        state.set(Artifact::Iso, 120);

        // ASSERT
        assert_eq!(state.current, 170);
        assert_eq!(state.artifacts.len(), 2);
    }
}
