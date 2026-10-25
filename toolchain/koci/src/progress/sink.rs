//! Byte progress sink.

use std::io::{self, Write};

#[cfg(feature = "copy")]
use super::{ByteSample, Phase, Reporter};

/// Destination of the byte samples produced by a [`Sink`].
pub(crate) trait Tracker {
    /// Records that `current` bytes were transferred in total.
    fn advance(&self, current: u64);
}

// A missing tracker, such as a download without progress reporting.
impl<T: Tracker> Tracker for Option<T> {
    fn advance(&self, current: u64) {
        if let Some(tracker) = self.as_ref() {
            tracker.advance(current);
        }
    }
}

/// [`Write`] adapter that reports the total bytes written so far.
pub(crate) struct Sink<'a, W, T> {
    inner: &'a mut W,
    tracker: T,
    current: u64,
}

impl<'a, W, T: Tracker> Sink<'a, W, T> {
    /// Wraps `inner`, starting the byte count at `current`.
    pub(crate) fn new(inner: &'a mut W, tracker: T, current: u64) -> Self {
        Self {
            inner,
            tracker,
            current,
        }
    }
}

impl<W: Write, T: Tracker> Write for Sink<'_, W, T> {
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

/// Byte samples forwarded to a [`Reporter`].
#[cfg(feature = "copy")]
pub(crate) struct Reported<'a> {
    reporter: &'a dyn Reporter,
    phase: Phase,
    index: usize,
    total: u64,
}

#[cfg(feature = "copy")]
impl<'a> Reported<'a> {
    /// Reports bytes for item `index`, declared as `total` bytes, in `phase`.
    pub(crate) fn new(reporter: &'a dyn Reporter, phase: Phase, index: usize, total: u64) -> Self {
        Self {
            reporter,
            phase,
            index,
            total,
        }
    }
}

#[cfg(feature = "copy")]
impl Tracker for Reported<'_> {
    fn advance(&self, current: u64) {
        self.reporter.bytes(ByteSample {
            phase: self.phase,
            index: Some(self.index),
            current,
            total: self.total,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    #[cfg(feature = "copy")]
    use std::sync::Mutex;

    use super::*;
    #[cfg(feature = "copy")]
    use crate::progress::Event;

    struct Counter<'a>(&'a Cell<u64>);

    impl Tracker for Counter<'_> {
        fn advance(&self, current: u64) {
            self.0.set(current);
        }
    }

    #[cfg(feature = "copy")]
    #[derive(Default)]
    struct Reports(Mutex<u64>);

    #[cfg(feature = "copy")]
    impl Reporter for Reports {
        fn report(&self, _event: Event<'_>) {
            let mut reports = self.0.lock().expect("unpoisoned reporter");
            *reports = reports.saturating_add(1);
        }
    }

    #[test]
    fn writes_advance_the_tracker_and_pass_bytes_through() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut inner = Vec::new();
        let mut sink = Sink::new(&mut inner, Counter(&counted), 0);

        // ACT
        sink.write_all(b"hello").expect("write into the sink");

        // ASSERT
        assert_eq!(counted.get(), 5);
        assert_eq!(inner, b"hello");
    }

    #[test]
    fn resumed_sinks_count_from_their_offset() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut inner = Vec::new();
        let mut sink = Sink::new(&mut inner, Counter(&counted), 10);

        // ACT
        sink.write_all(b"abc").expect("write into the sink");

        // ASSERT
        assert_eq!(counted.get(), 13);
    }

    #[test]
    fn a_missing_tracker_still_streams_bytes() {
        // ARRANGE
        let mut inner = Vec::new();
        let mut sink = Sink::new(&mut inner, Option::<Counter<'_>>::None, 0);

        // ACT
        sink.write_all(b"abc").expect("write into the sink");

        // ASSERT
        assert_eq!(inner, b"abc");
    }

    #[cfg(feature = "copy")]
    #[test]
    fn reported_sink_emits_one_event_per_write() {
        // ARRANGE
        let reporter = Reports::default();
        let mut inner = Vec::new();
        let mut sink = Sink::new(
            &mut inner,
            Reported::new(&reporter, Phase::CopyBlobs, 2, 1024),
            0,
        );

        // ACT
        sink.write_all(b"abcd").expect("write into the sink");

        // ASSERT
        assert_eq!(inner, b"abcd");
        assert_eq!(*reporter.0.lock().expect("unpoisoned reporter"), 1);
    }
}
