//! Forwards wizard build progress to the update preparation stream.

use tokio::sync::mpsc;
use wizard::progress::{Event, Reporter};

use crate::ipc::proto::provision::PrepareUpdateProgress;

// Bytes in one mebibyte.
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Reporter that turns wizard events into update preparation messages.
pub(crate) struct Forwarder {
    sender: mpsc::Sender<PrepareUpdateProgress>,
}

impl Forwarder {
    /// Creates a reporter writing to `sender`.
    pub(crate) const fn new(sender: mpsc::Sender<PrepareUpdateProgress>) -> Self {
        Self { sender }
    }

    fn send(&self, message: &str) {
        let _sent = self.sender.try_send(PrepareUpdateProgress {
            message: message.to_owned(),
            ..Default::default()
        });
    }
}

impl Reporter for Forwarder {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { label } | Event::Completed { label } => self.send(label),
            Event::Sized { artifacts, bytes } => self.send(&sized_message(artifacts, bytes)),
            Event::ArtifactBytes { .. } | Event::Note { detail: true, .. } => {}
            Event::Node { node, state } => self.send(&format!("{node} {}", state.title())),
            Event::Note {
                message,
                detail: false,
                ..
            } => self.send(message),
        }
    }
}

fn sized_message(artifacts: usize, bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => format!("{artifacts} artifacts, {} MiB total", mib(bytes)),
        None => format!("{artifacts} artifacts"),
    }
}

fn mib(bytes: u64) -> u64 {
    bytes.checked_div(BYTES_PER_MIB).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;
    use wizard::progress::{Event, NodeState, Reporter as _};

    use super::Forwarder;

    type Channel = (
        mpsc::Sender<super::PrepareUpdateProgress>,
        mpsc::Receiver<super::PrepareUpdateProgress>,
    );

    fn channel() -> Channel {
        mpsc::channel(4)
    }

    fn next_message(receiver: &mut mpsc::Receiver<super::PrepareUpdateProgress>) -> Option<String> {
        receiver.try_recv().ok().map(|progress| progress.message)
    }

    #[test]
    fn started_label_becomes_message() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Started {
            label: "Resolving sources",
        });

        // ASSERT
        assert_eq!(
            next_message(&mut receiver).as_deref(),
            Some("Resolving sources")
        );
    }

    #[test]
    fn sized_event_names_artifacts_and_total_size() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Sized {
            artifacts: 3,
            bytes: Some(5 * super::BYTES_PER_MIB),
        });
        forwarder.report(Event::Sized {
            artifacts: 2,
            bytes: None,
        });

        // ASSERT
        assert_eq!(
            next_message(&mut receiver).as_deref(),
            Some("3 artifacts, 5 MiB total")
        );
        assert_eq!(next_message(&mut receiver).as_deref(), Some("2 artifacts"));
    }

    #[test]
    fn node_event_combines_name_and_state() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Node {
            node: "kernel",
            state: NodeState::Completed,
        });

        // ASSERT
        assert_eq!(next_message(&mut receiver).as_deref(), Some("kernel done"));
    }

    #[test]
    fn detail_note_is_dropped_and_plain_note_is_forwarded() {
        // ARRANGE
        let (sender, mut receiver) = channel();
        let forwarder = Forwarder::new(sender);

        // ACT
        forwarder.report(Event::Note {
            node: Some("kernel"),
            message: "sha256:abc 2048 bytes",
            detail: true,
        });
        forwarder.report(Event::Note {
            node: None,
            message: "Downloading layers",
            detail: false,
        });

        // ASSERT
        assert_eq!(
            next_message(&mut receiver).as_deref(),
            Some("Downloading layers")
        );
        assert_eq!(next_message(&mut receiver), None);
    }

    #[test]
    fn report_inside_runtime_block_on_forwards_message() {
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
            next_message(&mut receiver).as_deref(),
            Some("kernel started")
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
            next_message(&mut receiver).as_deref(),
            Some("kernel started")
        );
        assert_eq!(next_message(&mut receiver), None);
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
