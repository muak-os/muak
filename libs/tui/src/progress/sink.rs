//! Byte-counting [`Write`] adapter.

use std::io::{self, Write};

/// Destination of the byte samples produced by a [`Sink`].
pub trait Tracker {
    /// Records that `current` bytes were transferred in total.
    fn advance(&self, current: u64);
}

// A missing tracker, such as a transfer without progress reporting.
impl<T: Tracker> Tracker for Option<T> {
    fn advance(&self, current: u64) {
        if let Some(tracker) = self.as_ref() {
            tracker.advance(current);
        }
    }
}

/// [`Write`] adapter that reports the total bytes written so far.
pub struct Sink<W, T> {
    inner: W,
    tracker: T,
    current: u64,
}

impl<W, T: Tracker> Sink<W, T> {
    /// Wraps `inner`, starting the byte count at `current`.
    #[must_use = "dropping it discards the wrapped writer"]
    pub const fn new(inner: W, tracker: T, current: u64) -> Self {
        Self {
            inner,
            tracker,
            current,
        }
    }

    /// Returns the wrapped writer, dropping the tracker and the byte count.
    #[must_use = "callers need the wrapped writer back"]
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write, T: Tracker> Write for Sink<W, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        if written == 0 {
            return Ok(written);
        }

        self.current = self
            .current
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        self.tracker.advance(self.current);

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
