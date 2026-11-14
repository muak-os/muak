//! Live step display with per-item byte aggregation and verbose notes.

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::live::lines::artifacts_message;
use crate::live::steps::Handle;

/// Live step display that aggregates per-item bytes and gates detail notes.
pub struct Tracked {
    steps: Handle,
    verbose: bool,
    bytes: Mutex<ItemBytes>,
}

impl Tracked {
    /// Creates a tracker over `steps` that shows detail only when `verbose`.
    #[must_use = "dropping it discards the tracked progress"]
    pub fn new(steps: Handle, verbose: bool) -> Self {
        Self {
            steps,
            verbose,
            bytes: Mutex::new(ItemBytes::default()),
        }
    }

    /// Begins a new step.
    pub fn start<M: Into<String>>(&self, message: M) {
        self.steps.start(message);
    }

    /// Freezes the active step as a success.
    pub fn complete<M: Into<String>>(&self, message: M) {
        self.steps.complete(message);
    }

    /// Updates the active step label.
    pub fn set_label<M: Into<String>>(&self, message: M) {
        self.steps.set_label(message);
    }

    /// Updates the active progress value.
    pub fn progress(&self, current: u64) {
        self.steps.progress(current);
    }

    /// Begins a byte step of `total` bytes, discarding previous byte samples.
    pub fn start_bytes<M: Into<String>>(&self, message: M, total: u64) {
        self.reset();
        self.steps.start_bytes(message, total);
    }

    /// Begins an item step of `total` items, discarding previous byte samples.
    pub fn start_items<M: Into<String>>(&self, message: M, total: u64) {
        self.reset();
        self.steps.start_items(message, total);
    }

    /// Begins a byte step of `bytes` when known, a spinner otherwise.
    ///
    /// Previous byte samples are discarded either way.
    pub fn sized<M: Into<String>>(&self, message: M, bytes: Option<u64>) {
        if let Some(bytes) = bytes {
            self.start_bytes(message, bytes);
            return;
        }

        self.reset();
        self.steps.start(message);
    }

    /// Begins a step labelled after the `count` artifacts being built.
    pub fn sized_artifacts(&self, count: usize, bytes: Option<u64>) {
        self.sized(artifacts_message(count), bytes);
    }

    /// Records the final size of the 1-based item at `index`.
    pub fn set_size(&self, index: usize, size: u64) {
        self.lock().set_size(index, size);
    }

    /// Reports item progress, aggregating samples when `index` is known.
    ///
    /// `index`, when present, is a 1-based item index. An indexed sample
    /// progresses by the cumulative bytes of every item, while an unindexed
    /// one progresses by `current` clamped to `total`.
    pub fn bytes(&self, index: Option<usize>, current: u64, total: u64) {
        let Some(index) = index else {
            self.steps.progress(current.min(total));
            return;
        };

        let current = self.lock().sample(index, current, total);
        self.steps.progress(current);
    }

    /// Marks the 1-based item at `index` as done and progresses to its total.
    pub fn item_done(&self, index: usize) {
        let current = self.lock().finish(index);
        self.steps.progress(current);
    }

    /// Freezes `message` above the active step.
    pub fn note<M: Into<String>>(&self, message: M) {
        self.steps.note(message);
    }

    /// Freezes `message` above the active step only when verbose.
    pub fn note_if_verbose<M: Into<String>>(&self, message: M) {
        if self.verbose {
            self.steps.note(message);
        }
    }

    /// Freezes `message` above the active step, prefixed by `node` when known.
    ///
    /// A `detail` note is shown only when verbose.
    pub fn note_detail(&self, node: Option<&str>, message: &str, detail: bool) {
        if detail && !self.verbose {
            return;
        }

        match node {
            Some(node) => self.steps.note(format!("{node}: {message}")),
            None => self.steps.note(message.to_owned()),
        }
    }

    /// Freezes `node: state` above the active step when verbose or `force`d.
    pub fn note_state(&self, node: &str, state: &str, force: bool) {
        if self.verbose || force {
            self.steps.note(format!("{node}: {state}"));
        }
    }

    fn lock(&self) -> MutexGuard<'_, ItemBytes> {
        self.bytes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reset(&self) {
        self.lock().reset();
    }
}

// Per-item byte aggregation addressed by 1-based indexes.
#[derive(Default)]
struct ItemBytes {
    sizes: Vec<u64>,
    downloaded: Vec<u64>,
    current: u64,
}

impl ItemBytes {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn set_size(&mut self, index: usize, size: u64) {
        debug_assert!(index > 0, "item indexes are 1-based");
        self.ensure(index);
        let Some(slot) = self.sizes.get_mut(index.saturating_sub(1)) else {
            return;
        };
        *slot = size;
    }

    fn sample(&mut self, index: usize, downloaded: u64, total: u64) -> u64 {
        self.set_size(index, total);
        let Some(slot) = self.downloaded.get_mut(index.saturating_sub(1)) else {
            return self.current;
        };
        let previous = *slot;
        *slot = downloaded;
        self.current = self
            .current
            .saturating_sub(previous)
            .saturating_add(downloaded);

        self.current
    }

    fn finish(&mut self, index: usize) -> u64 {
        let size = self.size(index);
        self.sample(index, size, size)
    }

    fn size(&self, index: usize) -> u64 {
        self.sizes
            .get(index.saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }

    fn ensure(&mut self, index: usize) {
        while self.sizes.len() < index {
            self.sizes.push(0);
            self.downloaded.push(0);
        }
    }
}
