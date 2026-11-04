//! Streaming command-line customization with sizes computable from the TOC.

use std::io::{self, Read, Write};

/// Accounts for the additional parameters and their separating spaces.
pub(super) fn size(source_size: u64, parameters: &[String]) -> io::Result<u64> {
    let mut size = source_size;
    for (index, parameter) in parameters.iter().enumerate() {
        let len = u64::try_from(parameter.len()).map_err(io::Error::other)?;
        let separator = u64::from(source_size != 0 || index != 0);
        size = size
            .checked_add(len)
            .and_then(|size| size.checked_add(separator))
            .ok_or_else(|| io::Error::other("kernel cmdline size overflow"))?;
    }

    Ok(size)
}

/// Inserts parameters before an optional final newline or NUL without buffering
/// the kernel package's command line or reading it during preflight.
pub(super) fn write(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    source_size: u64,
    parameters: &str,
) -> io::Result<()> {
    let prefix = source_size.saturating_sub(1);
    if io::copy(&mut (&mut *reader).take(prefix), writer)? != prefix {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }

    let mut last = [0_u8];
    let mut terminator = false;
    if source_size != 0 {
        reader.read_exact(&mut last)?;
        terminator = matches!(last, [b'\n' | b'\r' | 0]);
        if !terminator {
            writer.write_all(&last)?;
        }
        writer.write_all(b" ")?;
    }
    writer.write_all(parameters.as_bytes())?;
    if terminator {
        writer.write_all(&last)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn appends_before_the_optional_terminator_with_the_exact_planned_size() {
        // ARRANGE
        let parameters = vec!["loglevel=7".to_owned(), "console=tty0".to_owned()];
        for (source, expected) in [
            (
                b"quiet".as_slice(),
                b"quiet loglevel=7 console=tty0".as_slice(),
            ),
            (b"quiet\n", b"quiet loglevel=7 console=tty0\n"),
            (b"quiet\r", b"quiet loglevel=7 console=tty0\r"),
            (b"quiet\0", b"quiet loglevel=7 console=tty0\0"),
            (b"", b"loglevel=7 console=tty0"),
            (b"\n", b" loglevel=7 console=tty0\n"),
        ] {
            let source_size = u64::try_from(source.len()).expect("source size");
            let mut output = Vec::new();

            // ACT
            write(
                &mut Cursor::new(source),
                &mut output,
                source_size,
                &parameters.join(" "),
            )
            .expect("customize");

            // ASSERT
            assert_eq!(output, expected);
            assert_eq!(
                u64::try_from(output.len()).expect("output size"),
                size(source_size, &parameters).expect("planned size")
            );
        }
    }

    #[test]
    fn rejects_truncated_source_and_size_overflow() {
        // ARRANGE / ACT / ASSERT
        let error = write(&mut Cursor::new(b"quiet"), &mut Vec::new(), 9, "debug")
            .expect_err("truncated source");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        size(u64::MAX, &["debug".to_owned()]).expect_err("size overflow");
    }
}
