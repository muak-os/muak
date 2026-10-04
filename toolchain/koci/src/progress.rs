//! Pull progress reporting hooks.

/// Observer of pull progress, replacing console noise with structured events.
pub trait Progress: Send + Sync {
    /// A pull of `reference` for `arch` started.
    fn pulling(&self, _reference: &str, _arch: &str) {}

    /// The platform manifest resolved to `layers` layers.
    fn resolved(&self, _layers: usize) {}

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

/// Progress observer writing human-readable lines to stderr.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stderr;

impl Progress for Stderr {
    fn pulling(&self, reference: &str, arch: &str) {
        eprintln!("Pulling {reference} for {arch}");
    }

    fn resolved(&self, layers: usize) {
        eprintln!("Resolved {layers} layer(s)");
    }

    fn layer_download_queued(&self, index: usize, total: usize, digest: &str) {
        eprintln!("Downloading layer {index}/{total}: {digest}");
    }

    fn layer_downloaded(&self, index: usize, total: usize, digest: &str) {
        eprintln!("Downloaded layer {index}/{total}: {digest}");
    }

    fn layer_extracting(&self, index: usize, total: usize, digest: &str) {
        eprintln!("Extracting layer {index}/{total}: {digest}");
    }
}
