//! Rreads once, writes every chunk to all downstream writers.

use std::io::{self, Read, Write};

/// Copies `input` to every output writer, preserving backpressure.
pub(crate) fn copy_to_all(
    input: &mut dyn Read,
    outputs: &mut [&mut (dyn Write + Send)],
) -> io::Result<u64> {
    let mut copied = 0_u64;
    let mut buf = vec![0_u8; 65536];

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).unwrap_or_default();
        for output in outputs.iter_mut() {
            output.write_all(chunk)?;
        }
        copied = copied.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    }

    for output in outputs.iter_mut() {
        output.flush()?;
    }

    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_identical_bytes_to_all_outputs() {
        // ARRANGE
        let mut input: &[u8] = b"hello fanout";
        let mut first = Vec::new();
        let mut second = Vec::new();
        let mut outputs: [&mut (dyn Write + Send); 2] = [&mut first, &mut second];

        // ACT
        let copied = copy_to_all(&mut input, &mut outputs).expect("copy");

        // ASSERT
        assert_eq!(copied, 12);
        assert_eq!(first, b"hello fanout");
        assert_eq!(second, b"hello fanout");
    }

    #[test]
    fn propagates_writer_errors() {
        // ARRANGE
        let mut input: &[u8] = b"data";
        let mut failing = FailingWriter;
        let mut outputs: [&mut (dyn Write + Send); 1] = [&mut failing];

        // ACT
        let result = copy_to_all(&mut input, &mut outputs);

        // ASSERT
        result.unwrap_err();
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
