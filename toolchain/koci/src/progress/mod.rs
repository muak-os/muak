//! Structured progress reporting.

pub mod report;
pub(crate) mod sink;

/// A long-running registry operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Pulling an image into a local tree.
    Pull,
    /// Copying an image between registries.
    Copy,
    /// Pushing a scratch image.
    Push,
    /// Repacking a published image.
    Repack,
}

impl Operation {
    /// Human-readable operation title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Pull => "koci pull",
            Self::Copy => "koci copy",
            Self::Push => "koci push",
            Self::Repack => "koci repack",
        }
    }
}

/// A progress phase inside an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Downloading layers.
    Download,
    /// Scanning layers for whiteouts.
    Scan,
    /// Extracting layers.
    Extract,
    /// Copying blobs between registries.
    CopyBlobs,
    /// Uploading a blob.
    Upload,
    /// Repacking platforms.
    RepackPlatforms,
}

impl Phase {
    /// Human-readable phase title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Download => "Downloading layers",
            Self::Scan => "Scanning layers",
            Self::Extract => "Extracting layers",
            Self::CopyBlobs => "Copying blobs",
            Self::Upload => "Uploading layer",
            Self::RepackPlatforms => "Repacking platforms",
        }
    }
}

/// Unit of a progress value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Byte counts.
    Bytes,
    /// Item counts.
    Items,
}

/// One progress event emitted by a long-running operation.
#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    /// An operation started.
    Started {
        /// Operation being performed.
        operation: Operation,
        /// Operation subject, such as an image reference.
        subject: &'a str,
    },
    /// A pull resolved a platform manifest.
    Resolved {
        /// Number of layers.
        layers: usize,
        /// Platform manifest digest.
        digest: &'a str,
    },
    /// A copy collected its manifest tree.
    Collected {
        /// Number of manifests.
        manifests: usize,
        /// Number of blobs.
        blobs: usize,
    },
    /// A push packed its layer.
    Packed {
        /// Number of entries.
        entries: usize,
        /// Layer size in bytes.
        bytes: u64,
    },
    /// A repack fetched an index.
    IndexFetched {
        /// Number of platforms.
        platforms: usize,
    },
    /// A phase started with a progress bar.
    PhaseStarted {
        /// Phase that started.
        phase: Phase,
        /// Phase label.
        label: &'a str,
        /// Total amount.
        total: u64,
        /// Unit of the total.
        unit: Unit,
    },
    /// A phase completed.
    PhaseCompleted {
        /// Phase that completed.
        phase: Phase,
        /// Completion label.
        label: &'a str,
    },
    /// Item progress advanced.
    Progress {
        /// Phase being advanced.
        phase: Phase,
        /// Completed items.
        current: u64,
        /// Total items.
        total: u64,
    },
    /// Byte progress advanced.
    Bytes {
        /// Phase being advanced.
        phase: Phase,
        /// Optional item index.
        index: Option<usize>,
        /// Bytes transferred.
        current: u64,
        /// Total bytes.
        total: u64,
    },
    /// An item was queued with its declared size.
    ItemQueued {
        /// Phase the item belongs to.
        phase: Phase,
        /// One-based item index.
        index: usize,
        /// Total items.
        total: usize,
        /// Item digest.
        digest: &'a str,
        /// Declared size.
        size: u64,
    },
    /// An item was skipped because the destination already had it.
    ItemSkipped {
        /// Phase the item belongs to.
        phase: Phase,
        /// Item digest.
        digest: &'a str,
    },
    /// An item completed.
    ItemDone {
        /// Phase the item belongs to.
        phase: Phase,
        /// One-based item index.
        index: usize,
        /// Item size.
        size: u64,
        /// Item digest.
        digest: &'a str,
    },
    /// A manifest was pushed.
    ManifestPushed {
        /// Manifest tag or digest.
        reference: &'a str,
    },
    /// A destination was verified.
    Verified {
        /// Verified digest.
        digest: &'a str,
    },
    /// A verbose detail note.
    Note {
        /// Note text.
        message: &'a str,
    },
    /// An operation completed.
    Done {
        /// Result digest.
        digest: &'a str,
        /// Published tags.
        tags: &'a [String],
    },
}

/// One byte progress sample for a phase item.
#[derive(Clone, Copy, Debug)]
pub struct ByteSample {
    /// Phase being advanced.
    pub phase: Phase,
    /// Optional item index.
    pub index: Option<usize>,
    /// Bytes transferred.
    pub current: u64,
    /// Total bytes.
    pub total: u64,
}

impl ByteSample {
    /// Builds the progress event for this sample.
    pub(crate) const fn event(self) -> Event<'static> {
        Event::Bytes {
            phase: self.phase,
            index: self.index,
            current: self.current,
            total: self.total,
        }
    }
}

/// Observer of structured progress events.
pub trait Reporter: Send + Sync {
    /// Reports one byte progress sample.
    fn bytes(&self, sample: ByteSample) {
        self.report(sample.event());
    }

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
    fn byte_sample_builds_a_byte_event() {
        // ARRANGE
        let sample = ByteSample {
            phase: Phase::Upload,
            index: None,
            current: 7,
            total: 10,
        };

        // ACT
        let event = sample.event();

        // ASSERT
        assert!(matches!(
            event,
            Event::Bytes {
                phase: Phase::Upload,
                index: None,
                current: 7,
                total: 10,
            }
        ));
    }
}
