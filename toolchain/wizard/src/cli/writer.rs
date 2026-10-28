//! Artifact target that reports written bytes.

use std::fs::File;
use std::io::{self, Write};

use wizard::artifact::Artifact;
use wizard::progress::{Event, Reporter};

/// Artifact target that reports the bytes it receives.
pub(super) struct CountingWriter<'a> {
    file: File,
    artifact: Artifact,
    reporter: &'a dyn Reporter,
    written: u64,
}

impl<'a> CountingWriter<'a> {
    /// Wraps `file` so every write is reported for `artifact`.
    pub(super) const fn new(file: File, artifact: Artifact, reporter: &'a dyn Reporter) -> Self {
        Self {
            file,
            artifact,
            reporter,
            written: 0,
        }
    }
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        if written == 0 {
            return Ok(written);
        }

        self.written = self
            .written
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        self.reporter.report(Event::ArtifactBytes {
            artifact: self.artifact,
            current: self.written,
        });

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<String>>);

    impl Reporter for Recorded {
        fn report(&self, event: Event<'_>) {
            self.0
                .lock()
                .expect("unpoisoned")
                .push(format!("{event:?}"));
        }
    }

    #[test]
    fn reports_cumulative_bytes_per_write() {
        // ARRANGE
        let recorded = Recorded::default();
        let file = tempfile::tempfile().expect("temp file");
        let mut writer = CountingWriter::new(file, Artifact::Uki, &recorded);

        // ACT
        writer.write_all(b"hello").expect("write");

        // ASSERT
        let events = recorded.0.lock().expect("unpoisoned");
        assert_eq!(events.len(), 1, "events: {events:?}");
        let event = events.first().expect("one byte event");
        assert!(event.contains("Uki"), "events: {events:?}");
        assert!(event.contains("current: 5"), "events: {events:?}");
    }
}
