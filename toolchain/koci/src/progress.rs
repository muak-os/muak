//! Pull progress reporting hooks.

/// Observer of pull progress, replacing console noise with structured events.
pub trait Progress: Send + Sync {
    /// A pull of `reference` for `arch` started.
    fn pulling(&self, _reference: &str, _arch: &str) {}

    /// The platform manifest resolved to `layers` layers.
    fn resolved(&self, _layers: usize) {}

    /// Layer `index` of `total` is being pre-scanned for whiteout markers.
    fn layer_scanning(&self, _index: usize, _total: usize, _digest: &str) {}

    /// Layer `index` of `total` was queued for download.
    fn layer_download_queued(&self, _index: usize, _total: usize, _digest: &str) {}

    /// Layer `index` of `total` finished downloading.
    fn layer_downloaded(&self, _index: usize, _total: usize, _digest: &str) {}

    /// Extraction of layer `index` of `total` started.
    fn layer_extracting(&self, _index: usize, _total: usize, _digest: &str) {}
}

/// Progress observer that ignores every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct Noop;

impl Progress for Noop {}
