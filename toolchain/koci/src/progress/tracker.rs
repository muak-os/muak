//! Byte samples forwarded from a sink to a reporter.

use tui::progress::sink::Tracker;

use super::{ByteSample, Phase, Reporter};

/// Byte samples forwarded to a [`Reporter`].
pub(crate) struct Reported<'a> {
    reporter: &'a dyn Reporter,
    phase: Phase,
    index: usize,
    total: u64,
}

impl<'a> Reported<'a> {
    /// Reports bytes for item `index`, declared as `total` bytes, in `phase`.
    pub(crate) const fn new(
        reporter: &'a dyn Reporter,
        phase: Phase,
        index: usize,
        total: u64,
    ) -> Self {
        Self {
            reporter,
            phase,
            index,
            total,
        }
    }
}

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
    use std::io::Write as _;
    use std::sync::Mutex;

    use tui::progress::sink::Sink;

    use super::*;
    use crate::progress::Event;

    #[derive(Default)]
    struct Reports(Mutex<u64>);

    impl Reporter for Reports {
        fn report(&self, _event: Event<'_>) {
            let mut reports = self.0.lock().expect("unpoisoned reporter");
            *reports = reports.saturating_add(1);
        }
    }

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
