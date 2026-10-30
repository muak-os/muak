//! Structured progress reporting for the build pipeline.

pub(crate) mod koci;
pub(crate) mod resolution;

use crate::artifact::Artifact;

/// Shared [`Noop`] reporter for requests without progress reporting.
pub const NOOP: Noop = Noop;

/// State of one pipeline node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// The node started running.
    Started,
    /// The node finished successfully.
    Completed,
    /// The node failed.
    Failed,
}

impl NodeState {
    /// Human-readable verb for this state.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "done",
            Self::Failed => "failed",
        }
    }
}

/// One progress event emitted by the build pipeline.
#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    /// A pipeline stage started.
    Started {
        /// Stage label.
        label: &'a str,
    },
    /// A pipeline stage completed.
    Completed {
        /// Completion label.
        label: &'a str,
    },
    /// The artifact targets are known.
    Sized {
        /// Number of artifact targets.
        artifacts: usize,
        /// Total bytes the targets will receive, when every final size is known.
        bytes: Option<u64>,
    },
    /// `current` bytes were written to the target of `artifact`.
    ArtifactBytes {
        /// Artifact being written.
        artifact: Artifact,
        /// Bytes written to the artifact so far.
        current: u64,
    },
    /// A pipeline node changed state.
    Node {
        /// Node name.
        node: &'a str,
        /// New node state.
        state: NodeState,
    },
    /// A detail line attributed to a node, when known.
    Note {
        /// Node the note belongs to, when any.
        node: Option<&'a str>,
        /// Note text.
        message: &'a str,
        /// Whether the note is per-item detail shown only when verbose.
        detail: bool,
    },
}

/// Observer of build pipeline progress events.
pub trait Reporter: Send + Sync {
    /// Reports one event.
    fn report(&self, event: Event<'_>);
}

/// Reporter that ignores every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct Noop;

impl Reporter for Noop {
    fn report(&self, _event: Event<'_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_states_have_verbs() {
        // ARRANGE
        let states = [
            (NodeState::Started, "started"),
            (NodeState::Completed, "done"),
            (NodeState::Failed, "failed"),
        ];

        // ACT & ASSERT
        for (state, title) in states {
            assert_eq!(state.title(), title);
        }
    }
}
