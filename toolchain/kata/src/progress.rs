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

#[cfg(test)]
mod tests {
    use std::mem::size_of_val;
    use std::path::Path;
    use std::sync::Mutex;

    use super::{Event, Noop, Reporter};

    #[derive(Debug, Default)]
    struct Recorder {
        labels: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn labels(&self) -> Vec<String> {
            self.labels.lock().expect("recorder lock").clone()
        }
    }

    impl Reporter for Recorder {
        fn report(&self, event: Event<'_>) {
            let label = match event {
                Event::Composed { release, entries } => format!("composed {release} {entries}"),
                Event::Wrote { path } => format!("wrote {}", path.display()),
                Event::Published { reference, digest } => format!("published {reference} {digest}"),
                Event::ChannelMoved {
                    image,
                    channel,
                    release,
                } => format!("channel {image} {channel} {release}"),
                Event::Verified { line, entries } => format!("verified {line} {entries}"),
                Event::Removed { identity, digest } => format!("removed {identity} {digest}"),
                Event::Upload { current, total } => format!("upload {current} {total}"),
                Event::LayerUploaded { digest, size } => format!("layer {digest} {size}"),
                Event::LayerSkipped { digest } => format!("skipped {digest}"),
            };
            self.labels.lock().expect("recorder lock").push(label);
        }
    }

    fn every_event() -> [Event<'static>; 9] {
        [
            Event::Composed {
                release: "1.0.0",
                entries: 3,
            },
            Event::Wrote {
                path: Path::new("catalog.toml"),
            },
            Event::Published {
                reference: "ghcr.io/muak-os/kata:1.0.0",
                digest: "sha256:aaaa",
            },
            Event::ChannelMoved {
                image: "ghcr.io/muak-os/kata",
                channel: "stable",
                release: "1.0.0",
            },
            Event::Verified {
                line: "1.0.0",
                entries: 3,
            },
            Event::Removed {
                identity: "x86_64/packages/core@1.0.0",
                digest: "sha256:bbbb",
            },
            Event::Upload {
                current: 64,
                total: 128,
            },
            Event::LayerUploaded {
                digest: "sha256:cccc",
                size: 4096,
            },
            Event::LayerSkipped {
                digest: "sha256:dddd",
            },
        ]
    }

    fn every_label() -> Vec<String> {
        [
            "composed 1.0.0 3",
            "wrote catalog.toml",
            "published ghcr.io/muak-os/kata:1.0.0 sha256:aaaa",
            "channel ghcr.io/muak-os/kata stable 1.0.0",
            "verified 1.0.0 3",
            "removed x86_64/packages/core@1.0.0 sha256:bbbb",
            "upload 64 128",
            "layer sha256:cccc 4096",
            "skipped sha256:dddd",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn recorder_preserves_event_order() {
        // ARRANGE
        let recorder = Recorder::default();

        // ACT
        for event in every_event() {
            recorder.report(event);
        }

        // ASSERT
        assert_eq!(recorder.labels(), every_label());
    }

    #[test]
    fn noop_accepts_every_variant() {
        // ARRANGE
        let reporter: &dyn Reporter = &Noop;

        // ACT
        for event in every_event() {
            reporter.report(event);
        }

        // ASSERT
        assert_eq!(size_of_val(reporter), 0);
    }
}
