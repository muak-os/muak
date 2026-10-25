//! Blob upload progress reporting.

/// Observer of a single blob upload.
pub trait BlobProgress: Send + Sync {
    /// The upload of `digest` with `total` bytes started.
    fn started(&self, _digest: &str, _total: u64) {}

    /// `sent` of `total` bytes were transferred.
    fn progress(&self, _sent: u64, _total: u64) {}

    /// The registry already holds `digest`, so the upload was skipped.
    fn skipped(&self, _digest: &str) {}

    /// The upload of `digest` completed.
    fn finished(&self, _digest: &str) {}
}

/// Blob progress observer that ignores every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct Noop;

impl BlobProgress for Noop {}
