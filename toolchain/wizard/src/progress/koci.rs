//! Adapter from koci progress events to wizard progress events.

use crate::progress::{Event, Reporter};

/// Wizard reporter scoped to one pipeline node, fed by koci.
pub(crate) struct Adapter<'a> {
    reporter: &'a dyn Reporter,
    node: &'static str,
}

impl<'a> Adapter<'a> {
    /// Wraps `reporter` so every event is attributed to `node`.
    pub(crate) const fn new(reporter: &'a dyn Reporter, node: &'static str) -> Self {
        Self { reporter, node }
    }

    fn note(&self, message: &str, detail: bool) {
        self.reporter.report(Event::Note {
            node: Some(self.node),
            message,
            detail,
        });
    }
}

impl koci::progress::Reporter for Adapter<'_> {
    fn report(&self, event: koci::progress::Event<'_>) {
        match event {
            koci::progress::Event::Started { operation, subject } => {
                self.note(&format!("{} {subject}", operation.title()), true);
            }
            koci::progress::Event::Resolved { layers, .. } => {
                self.note(&format!("resolved {layers} layers"), true);
            }
            koci::progress::Event::Collected { manifests, blobs } => {
                self.note(
                    &format!("collected {manifests} manifests, {blobs} blobs"),
                    true,
                );
            }
            koci::progress::Event::Packed { entries, bytes } => {
                self.note(&format!("packed {entries} entries ({bytes} bytes)"), true);
            }
            koci::progress::Event::IndexFetched { platforms } => {
                self.note(&format!("fetched index ({platforms} platforms)"), true);
            }
            koci::progress::Event::PhaseStarted { label, .. }
            | koci::progress::Event::PhaseCompleted { label, .. } => self.note(label, false),
            koci::progress::Event::Progress { .. }
            | koci::progress::Event::Bytes { .. }
            | koci::progress::Event::ItemQueued { .. }
            | koci::progress::Event::Done { .. } => {}
            koci::progress::Event::ItemSkipped { digest, .. } => {
                self.note(&format!("skipped existing {digest}"), true);
            }
            koci::progress::Event::ItemDone { size, digest, .. } => {
                self.note(&format!("{digest} {size} bytes"), true);
            }
            koci::progress::Event::ManifestPushed { reference } => {
                self.note(&format!("pushed manifest {reference}"), true);
            }
            koci::progress::Event::Verified { digest } => {
                self.note(&format!("verified destination digest {digest}"), false);
            }
            koci::progress::Event::Note { message } => self.note(message, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use koci::progress::Reporter as _;

    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<(Option<String>, String, bool)>>);

    impl Reporter for Recorded {
        fn report(&self, event: Event<'_>) {
            self.0.lock().expect("unpoisoned").extend(note(event));
        }
    }

    fn note(event: Event<'_>) -> Option<(Option<String>, String, bool)> {
        let Event::Note {
            node,
            message,
            detail,
        } = event
        else {
            return None;
        };

        Some((node.map(str::to_owned), message.to_owned(), detail))
    }

    #[test]
    fn phases_become_node_notes() {
        // ARRANGE
        let recorded = Recorded::default();
        let adapter = Adapter::new(&recorded, "kernel");

        // ACT
        adapter.report(koci::progress::Event::PhaseStarted {
            phase: koci::progress::Phase::Download,
            label: "Downloading layers",
            total: 1024,
            unit: koci::progress::Unit::Bytes,
        });
        adapter.report(koci::progress::Event::PhaseCompleted {
            phase: koci::progress::Phase::Download,
            label: "Downloaded 2/2 layers",
        });

        // ASSERT
        let notes = recorded.0.lock().expect("unpoisoned");
        assert_eq!(
            *notes,
            vec![
                (
                    Some("kernel".to_owned()),
                    "Downloading layers".to_owned(),
                    false
                ),
                (
                    Some("kernel".to_owned()),
                    "Downloaded 2/2 layers".to_owned(),
                    false
                ),
            ]
        );
    }

    #[test]
    fn item_detail_and_byte_samples_are_separated() {
        // ARRANGE
        let recorded = Recorded::default();
        let adapter = Adapter::new(&recorded, "stub");

        // ACT
        adapter.report(koci::progress::Event::ItemDone {
            phase: koci::progress::Phase::Download,
            index: 1,
            size: 2048,
            digest: "sha256:abc",
        });
        adapter.report(koci::progress::Event::Bytes {
            phase: koci::progress::Phase::Download,
            index: Some(1),
            current: 1024,
            total: 2048,
        });

        // ASSERT
        let notes = recorded.0.lock().expect("unpoisoned");
        assert_eq!(notes.len(), 1, "byte samples must not become notes");
        let note = notes.first().expect("one item note");
        assert_eq!(note.1, "sha256:abc 2048 bytes");
        assert!(note.2, "item detail must be verbose-only");
    }

    #[test]
    fn non_note_events_are_dropped() {
        // ARRANGE
        let recorded = Recorded::default();
        let adapter = Adapter::new(&recorded, "uki");

        // ACT
        adapter.report(koci::progress::Event::Done {
            digest: "sha256:abc",
            tags: &[],
        });

        // ASSERT
        assert!(recorded.0.lock().expect("unpoisoned").is_empty());
    }
}
