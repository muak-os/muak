//! Plain progress reporter.

use super::{Event, Reporter};

/// Plain CLI progress reporter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Plain {
    /// Whether per-item detail is enabled.
    pub verbose: bool,
}

/// Library progress reporter that prints every event.
pub type Stderr = Plain;

impl Plain {
    fn detail(self, message: &str) {
        if self.verbose {
            eprintln!("{message}");
        }
    }
}

impl Reporter for Plain {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { operation, subject } => {
                eprintln!("[..] {} {subject}", operation.title());
            }
            Event::Resolved { layers, .. } => {
                eprintln!("[ok] Resolved {layers} layers");
            }
            Event::Collected { manifests, blobs } => {
                eprintln!("[ok] Collected {manifests} manifests, {blobs} blobs");
            }
            Event::Packed { entries, bytes } => {
                eprintln!("[ok] Packed {entries} entries ({bytes} bytes)");
            }
            Event::IndexFetched { platforms } => {
                eprintln!("[ok] Fetched index ({platforms} platforms)");
            }
            Event::PhaseStarted { label, .. } => {
                eprintln!("[..] {label}");
            }
            Event::PhaseCompleted { label, .. } => {
                eprintln!("[ok] {label}");
            }
            Event::Progress { .. } | Event::Bytes { .. } | Event::Done { .. } => {}
            Event::ItemQueued {
                phase,
                index,
                total,
                digest,
                size,
            } => self.detail(&format!(
                "[..] {} {index}/{total} {digest} {size} bytes",
                phase.title()
            )),
            Event::ItemSkipped { digest, .. } => {
                self.detail(&format!("[ok] Skipped existing {digest}"));
            }
            Event::ItemDone { digest, size, .. } => {
                self.detail(&format!("[ok] {digest} {size} bytes"));
            }
            Event::ManifestPushed { reference } => {
                self.detail(&format!("[ok] Pushed manifest {reference}"));
            }
            Event::Verified { digest } => {
                eprintln!("[ok] Verified destination digest {digest}");
            }
            Event::Note { message } => self.detail(&format!("[..] {message}")),
        }
    }
}
