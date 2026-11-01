//! Integration tests for the byte-counting sink.

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};

    use tui::progress::sink::{Sink, Tracker};

    struct Counter<'a>(&'a Cell<u64>);

    impl Tracker for Counter<'_> {
        fn advance(&self, current: u64) {
            self.0.set(current);
        }
    }

    struct ZeroWriter;

    impl Write for ZeroWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct PartialWriter<'a>(&'a Cell<usize>);

    impl Write for PartialWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let accepted = buf.len().min(2);
            self.0.set(self.0.get().saturating_add(accepted));

            Ok(accepted)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("inner writer failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FlushWriter<'a>(&'a Cell<bool>);

    impl Write for FlushWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.set(true);

            Ok(())
        }
    }

    #[test]
    fn writes_pass_through_and_advance_the_tracker() {
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

    #[test]
    fn zero_byte_writes_do_not_advance_the_tracker() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut sink = Sink::new(ZeroWriter, Counter(&counted), 4);

        // ACT
        let written = sink.write(b"abc").expect("write into the sink");

        // ASSERT
        assert_eq!(written, 0);
        assert_eq!(counted.get(), 0);
    }

    #[test]
    fn flush_delegates_to_the_inner_writer() {
        // ARRANGE
        let counted = Cell::new(0);
        let flushed = Cell::new(false);
        let mut sink = Sink::new(FlushWriter(&flushed), Counter(&counted), 0);

        // ACT
        sink.flush().expect("flush the sink");

        // ASSERT
        assert!(flushed.get());
    }

    #[test]
    fn a_sink_can_own_its_writer() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut sink = Sink::new(Vec::new(), Counter(&counted), 0);
        sink.write_all(b"abcd").expect("write into the sink");

        // ACT
        let writer = sink.into_inner();

        // ASSERT
        assert_eq!(counted.get(), 4);
        assert_eq!(writer, b"abcd");
    }

    #[test]
    fn partial_writes_advance_by_the_returned_count() {
        // ARRANGE
        let counted = Cell::new(0);
        let accepted = Cell::new(0);
        let mut sink = Sink::new(PartialWriter(&accepted), Counter(&counted), 0);

        // ACT
        let written = sink.write(b"hello").expect("write into the sink");

        // ASSERT
        assert_eq!(written, 2);
        assert_eq!(accepted.get(), 2);
        assert_eq!(counted.get(), 2);
    }

    #[test]
    fn byte_counts_saturate_at_the_maximum() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut inner = Vec::new();
        let mut sink = Sink::new(&mut inner, Counter(&counted), u64::MAX);

        // ACT
        sink.write_all(b"abc").expect("write into the sink");

        // ASSERT
        assert_eq!(counted.get(), u64::MAX);
        assert_eq!(inner, b"abc");
    }

    #[test]
    fn inner_errors_do_not_advance_the_tracker() {
        // ARRANGE
        let counted = Cell::new(0);
        let mut sink = Sink::new(FailingWriter, Counter(&counted), 4);

        // ACT
        let error = sink.write(b"abc").expect_err("inner writer fails");

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(counted.get(), 0);
    }
}
