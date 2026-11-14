//! Renders prepare-update proto events on the live step display.

use tui::live::steps::Steps;
use tui::live::tracker::Tracked;

use crate::client::provision_service::PrepareNodeState;
use crate::client::provision_service::prepare_update_progress::Event;

/// Live step adapter for structured prepare-update progress events.
pub(super) struct Progress {
    tracked: Tracked,
}

impl Progress {
    /// Creates an adapter over `steps` that shows details only when `verbose`.
    pub(super) fn new(steps: &Steps, verbose: bool) -> Self {
        Self {
            tracked: Tracked::new(steps.handle(), verbose),
        }
    }

    /// Renders one structured progress event.
    pub(super) fn report(&self, event: Event) {
        match event {
            Event::Started(stage) => self.tracked.start(stage.label),
            Event::Completed(stage) => self.tracked.complete(stage.label),
            Event::Sized(sized) => self.tracked.sized_artifacts(
                usize::try_from(sized.artifacts).unwrap_or(usize::MAX),
                sized.bytes,
            ),
            Event::Node(node) => {
                let state =
                    PrepareNodeState::try_from(node.state).unwrap_or(PrepareNodeState::Unspecified);
                self.tracked.note_state(
                    &node.node,
                    node_title(state),
                    state == PrepareNodeState::Failed,
                );
            }
            Event::Note(note) => {
                self.tracked
                    .note_detail(note.node.as_deref(), &note.message, note.detail);
            }
            Event::ArtifactBytes(bytes) => {
                self.tracked.bytes(
                    Some(artifact_index(bytes.artifact_index)),
                    bytes.current,
                    bytes.current,
                );
            }
        }
    }
}

// Wire indexes are 0-based, display indexes are 1-based.
fn artifact_index(index: u32) -> usize {
    usize::try_from(index).unwrap_or(0).saturating_add(1)
}

fn node_title(state: PrepareNodeState) -> &'static str {
    match state {
        PrepareNodeState::Started => "started",
        PrepareNodeState::Completed => "done",
        PrepareNodeState::Failed => "failed",
        PrepareNodeState::Unspecified => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tui::live::Options;
    use tui::live::steps::Steps;
    use tui::screen::ansi::Ansi;
    use tui::size::Size;
    use tui::style::Theme;

    use super::{Progress, artifact_index, node_title};
    use crate::client::provision_service::prepare_update_progress::Event;
    use crate::client::provision_service::{
        PrepareNode, PrepareNodeState, PrepareNote, PrepareStageCompleted, PrepareStageStarted,
    };

    fn rendered(verbose: bool, report: impl FnOnce(&Progress)) -> String {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("steps");
        let file = fs::File::create(&path).expect("steps output file");
        let steps = Steps::new(
            Ansi::new(file, Size::new(80, 24)),
            Options::new(false, Theme::plain()),
        )
        .expect("steps");
        let progress = Progress::new(&steps, verbose);

        report(&progress);
        steps.finish().expect("finish");

        fs::read_to_string(&path).expect("read steps output")
    }

    #[test]
    fn stages_freeze_once_and_failed_nodes_show_without_verbose() {
        // ARRANGE
        let started = Event::Started(PrepareStageStarted {
            label: "Preparing artifacts".to_owned(),
        });
        let completed = Event::Completed(PrepareStageCompleted {
            label: "Prepared".to_owned(),
        });
        let failed = Event::Node(PrepareNode {
            node: "kernel".to_owned(),
            state: PrepareNodeState::Failed.into(),
        });

        // ACT
        let text = rendered(false, |progress| {
            progress.report(started);
            progress.report(completed);
            progress.report(failed);
        });

        // ASSERT
        assert_eq!(
            text.matches("Preparing artifacts").count(),
            1,
            "a stage must freeze once: {text}"
        );
        assert_eq!(
            text.matches("Prepared").count(),
            1,
            "a completion must freeze once: {text}"
        );
        assert!(text.contains("kernel: failed"), "output: {text}");
    }

    fn detail_note() -> Event {
        Event::Note(PrepareNote {
            node: Some("kernel".to_owned()),
            message: "sha256:abc 2048 bytes".to_owned(),
            detail: true,
        })
    }

    #[test]
    fn detail_notes_are_shown_only_when_verbose() {
        // ARRANGE
        let plain = Event::Note(PrepareNote {
            node: None,
            message: "resolved sources".to_owned(),
            detail: false,
        });

        // ACT
        let quiet = rendered(false, |progress| {
            progress.report(detail_note());
            progress.report(plain);
        });
        let verbose = rendered(true, |progress| progress.report(detail_note()));

        // ASSERT
        assert!(!quiet.contains("sha256:abc"), "output: {quiet}");
        assert!(quiet.contains("resolved sources"), "output: {quiet}");
        assert!(verbose.contains("kernel: sha256:abc"), "output: {verbose}");
    }

    #[test]
    fn artifact_indexes_are_one_based() {
        // ARRANGE
        let first = 0;

        // ACT
        let index = artifact_index(first);

        // ASSERT
        assert_eq!(index, 1);
    }

    #[test]
    fn node_titles_name_each_state() {
        // ARRANGE
        let states = [
            (PrepareNodeState::Started, "started"),
            (PrepareNodeState::Completed, "done"),
            (PrepareNodeState::Failed, "failed"),
            (PrepareNodeState::Unspecified, "unknown"),
        ];

        // ACT & ASSERT
        for (state, title) in states {
            assert_eq!(node_title(state), title);
        }
    }
}
