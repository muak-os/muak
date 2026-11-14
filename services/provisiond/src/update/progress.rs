//! Forwards wizard build progress to the update preparation stream.

use tokio::sync::mpsc;
use wizard::progress::{Event, NodeState, Reporter};

use crate::ipc::proto::provision::prepare_update_progress::Event as ProtoEvent;
use crate::ipc::proto::provision::{
    PrepareArtifactBytes, PrepareNode, PrepareNodeState, PrepareNote, PrepareSized,
    PrepareStageCompleted, PrepareStageStarted, PrepareUpdateProgress,
};

/// Reporter that turns wizard events into structured update preparation messages.
pub(crate) struct Forwarder {
    sender: mpsc::Sender<PrepareUpdateProgress>,
}

impl Forwarder {
    /// Creates a reporter writing to `sender`.
    pub(crate) const fn new(sender: mpsc::Sender<PrepareUpdateProgress>) -> Self {
        Self { sender }
    }
}

impl Reporter for Forwarder {
    fn report(&self, event: Event<'_>) {
        let _sent = self.sender.try_send(PrepareUpdateProgress {
            event: Some(convert(event)),
            ..Default::default()
        });
    }
}

fn convert(event: Event<'_>) -> ProtoEvent {
    match event {
        Event::Started { label } => ProtoEvent::Started(PrepareStageStarted {
            label: label.to_owned(),
        }),
        Event::Completed { label } => ProtoEvent::Completed(PrepareStageCompleted {
            label: label.to_owned(),
        }),
        Event::Sized { artifacts, bytes } => ProtoEvent::Sized(PrepareSized {
            artifacts: u32::try_from(artifacts).unwrap_or(u32::MAX),
            bytes,
        }),
        Event::ArtifactBytes { artifact, current } => {
            ProtoEvent::ArtifactBytes(PrepareArtifactBytes {
                artifact_index: u32::try_from(artifact.to_index()).unwrap_or(u32::MAX),
                current,
            })
        }
        Event::Node { node, state } => ProtoEvent::Node(PrepareNode {
            node: node.to_owned(),
            state: node_state(state).into(),
        }),
        Event::Note {
            node,
            message,
            detail,
        } => ProtoEvent::Note(PrepareNote {
            node: node.map(str::to_owned),
            message: message.to_owned(),
            detail,
        }),
    }
}

fn node_state(state: NodeState) -> PrepareNodeState {
    match state {
        NodeState::Started => PrepareNodeState::Started,
        NodeState::Completed => PrepareNodeState::Completed,
        NodeState::Failed => PrepareNodeState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;
    use wizard::artifact::Artifact;
    use wizard::progress::{Event, NodeState, Reporter as _};

    use super::Forwarder;
    use crate::ipc::proto::provision::prepare_update_progress::Event as ProtoEvent;
    use crate::ipc::proto::provision::{
        PrepareArtifactBytes, PrepareNode, PrepareNodeState, PrepareNote, PrepareSized,
        PrepareStageCompleted, PrepareStageStarted, PrepareUpdateProgress,
    };

    fn channel() -> (
        mpsc::Sender<PrepareUpdateProgress>,
        mpsc::Receiver<PrepareUpdateProgress>,
    ) {
        mpsc::channel(4)
    }

    fn next_event(receiver: &mut mpsc::Receiver<PrepareUpdateProgress>) -> Option<ProtoEvent> {
        let progress = receiver.try_recv().ok()?;

        progress.event
    }

    #[test]
    fn started_label_becomes_started_event() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Started {
            label: "Resolving sources",
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Started(PrepareStageStarted {
                label: "Resolving sources".to_owned(),
            }))
        );
    }

    #[test]
    fn completed_label_becomes_completed_event() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Completed {
            label: "Preparing artifacts",
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Completed(PrepareStageCompleted {
                label: "Preparing artifacts".to_owned(),
            }))
        );
    }

    #[test]
    fn sized_event_carries_artifacts_and_bytes() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Sized {
            artifacts: 3,
            bytes: Some(2048),
        });
        forwarder.report(Event::Sized {
            artifacts: 2,
            bytes: None,
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Sized(PrepareSized {
                artifacts: 3,
                bytes: Some(2048),
            }))
        );
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Sized(PrepareSized {
                artifacts: 2,
                bytes: None,
            }))
        );
    }

    #[test]
    fn node_event_carries_state() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Node {
            node: "kernel",
            state: NodeState::Failed,
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Node(PrepareNode {
                node: "kernel".to_owned(),
                state: PrepareNodeState::Failed.into(),
            }))
        );
    }

    #[test]
    fn detail_note_is_forwarded() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Note {
            node: Some("kernel"),
            message: "sha256:abc 2048 bytes",
            detail: true,
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Note(PrepareNote {
                node: Some("kernel".to_owned()),
                message: "sha256:abc 2048 bytes".to_owned(),
                detail: true,
            }))
        );
    }

    #[test]
    fn artifact_bytes_uses_zero_based_index() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::ArtifactBytes {
            artifact: Artifact::Iso,
            current: 1024,
        });

        // ASSERT
        let index = u32::try_from(Artifact::Iso.to_index()).expect("index fits in u32");
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::ArtifactBytes(PrepareArtifactBytes {
                artifact_index: index,
                current: 1024,
            }))
        );
    }

    #[test]
    fn report_inside_runtime_block_on_forwards_event() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        // ACT
        runtime.block_on(async {
            forwarder.report(Event::Node {
                node: "kernel",
                state: NodeState::Started,
            });
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Node(PrepareNode {
                node: "kernel".to_owned(),
                state: PrepareNodeState::Started.into(),
            }))
        );
    }

    #[test]
    fn full_channel_drops_message_instead_of_waiting() {
        // ARRANGE
        let (sender, mut receiver) = mpsc::channel(1);
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Node {
            node: "kernel",
            state: NodeState::Started,
        });
        forwarder.report(Event::Node {
            node: "kernel",
            state: NodeState::Completed,
        });

        // ASSERT
        assert_eq!(
            next_event(&mut receiver),
            Some(ProtoEvent::Node(PrepareNode {
                node: "kernel".to_owned(),
                state: PrepareNodeState::Started.into(),
            }))
        );
        assert_eq!(next_event(&mut receiver), None);
    }

    #[test]
    fn dropped_receiver_is_ignored() {
        // ARRANGE
        let (sender, receiver) = channel();
        let forwarder = Forwarder::new(sender);
        drop(receiver);

        // ACT
        forwarder.report(Event::Started {
            label: "Pulling update for release: v2",
        });
        forwarder.report(Event::Sized {
            artifacts: 3,
            bytes: None,
        });

        // ASSERT
        forwarder.report(Event::Node {
            node: "kernel",
            state: NodeState::Started,
        });
    }
}
