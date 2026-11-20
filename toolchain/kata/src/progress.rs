//! Structured catalog operation progress.

use std::path::Path;

/// One progress event emitted by a catalog operation.
#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    /// A release line was composed.
    Composed {
        /// Composed release line.
        release: &'a str,
        /// Number of planned entries.
        entries: usize,
    },
    /// A document was written to the catalog root.
    Wrote {
        /// Path of the written document.
        path: &'a Path,
    },
    /// A catalog image was published.
    Published {
        /// Published image reference.
        reference: &'a str,
        /// Digest of the pushed manifest.
        digest: &'a str,
    },
    /// A channel tag moved to a release.
    ChannelMoved {
        /// Catalog image without a tag.
        image: &'a str,
        /// Channel tag that moved.
        channel: &'a str,
        /// Release the channel now points at.
        release: &'a str,
    },
    /// A release line was verified against the registry.
    Verified {
        /// Verified release line.
        line: &'a str,
        /// Number of verified entries.
        entries: usize,
    },
    /// An entry was removed from a document.
    Removed {
        /// Removed entry identity.
        identity: &'a str,
        /// Digest the entry pinned.
        digest: &'a str,
    },
    /// Registry upload progress.
    Upload {
        /// Bytes uploaded.
        current: u64,
        /// Total bytes.
        total: u64,
    },
    /// A registry layer upload finished.
    LayerUploaded {
        /// Layer digest.
        digest: &'a str,
        /// Layer size in bytes.
        size: u64,
    },
    /// A registry layer upload was skipped because the blob already exists.
    LayerSkipped {
        /// Layer digest.
        digest: &'a str,
    },
}

/// Observer of catalog operation progress.
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

/// koci push reporter that forwards upload progress as catalog events.
pub(crate) struct Upload<'a>(pub(crate) &'a dyn Reporter);

impl koci::progress::Reporter for Upload<'_> {
    fn report(&self, event: koci::progress::Event<'_>) {
        match event {
            koci::progress::Event::Bytes { current, total, .. } => {
                self.0.report(Event::Upload { current, total });
            }
            koci::progress::Event::ItemDone { digest, size, .. } => {
                self.0.report(Event::LayerUploaded { digest, size });
            }
            koci::progress::Event::ItemSkipped { digest, .. } => {
                self.0.report(Event::LayerSkipped { digest });
            }
            koci::progress::Event::Started { .. }
            | koci::progress::Event::Resolved { .. }
            | koci::progress::Event::Collected { .. }
            | koci::progress::Event::Packed { .. }
            | koci::progress::Event::IndexFetched { .. }
            | koci::progress::Event::PhaseStarted { .. }
            | koci::progress::Event::PhaseCompleted { .. }
            | koci::progress::Event::Progress { .. }
            | koci::progress::Event::ItemQueued { .. }
            | koci::progress::Event::ManifestPushed { .. }
            | koci::progress::Event::Verified { .. }
            | koci::progress::Event::Note { .. }
            | koci::progress::Event::Done { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use koci::progress::Reporter as _;

    use super::{Event, Reporter, Upload};

    #[derive(Debug, Default)]
    struct Recorder {
        events: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn events(&self) -> Vec<String> {
            self.events.lock().expect("recorder lock").clone()
        }
    }

    impl Reporter for Recorder {
        fn report(&self, event: Event<'_>) {
            self.events
                .lock()
                .expect("recorder lock")
                .push(format!("{event:?}"));
        }
    }

    #[test]
    fn upload_forwards_upload_progress_and_ignores_the_rest() {
        // ARRANGE
        let recorder = Recorder::default();
        let upload = Upload(&recorder);

        // ACT
        upload.report(koci::progress::Event::Bytes {
            phase: koci::progress::Phase::Upload,
            index: None,
            current: 64,
            total: 128,
        });
        upload.report(koci::progress::Event::ItemDone {
            phase: koci::progress::Phase::Upload,
            index: 1,
            size: 4096,
            digest: "sha256:cccc",
        });
        upload.report(koci::progress::Event::ItemSkipped {
            phase: koci::progress::Phase::Upload,
            digest: "sha256:dddd",
        });
        upload.report(koci::progress::Event::Note { message: "ignored" });

        // ASSERT
        assert_eq!(
            recorder.events(),
            [
                "Upload { current: 64, total: 128 }",
                "LayerUploaded { digest: \"sha256:cccc\", size: 4096 }",
                "LayerSkipped { digest: \"sha256:dddd\" }",
            ]
            .map(str::to_owned)
            .to_vec()
        );
    }
}
