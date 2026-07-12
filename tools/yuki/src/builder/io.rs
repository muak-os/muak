//! Pure I/O primitives for streaming PE image bytes.

use std::io::{Read, Write};

use crate::error::{Result, YukiError};

const ZERO_BUF: [u8; 8192] = [0; 8192];

pub(crate) fn copy_exact<R: Read + ?Sized, W: Write>(
    reader: &mut R,
    writer: &mut W,
    len: u64,
    name: &'static str,
    on_chunk: &mut dyn FnMut(&[u8]),
) -> Result<()> {
    let mut buffer = [0_u8; 16384];
    let mut limited = reader.take(len);
    let mut copied = 0_u64;
    loop {
        let n = limited.read(&mut buffer).map_err(YukiError::Io)?;
        if n == 0 {
            break;
        }
        let Some(chunk) = buffer.get(..n) else {
            return Err(YukiError::Io(std::io::Error::other("buffer range invalid")));
        };
        writer.write_all(chunk)?;
        on_chunk(chunk);
        let Ok(amount) = u64::try_from(n) else {
            return Err(YukiError::InvalidPeStructure(
                "read count overflow".to_owned(),
            ));
        };
        copied = copied.saturating_add(amount);
    }

    if copied != len {
        return Err(YukiError::InvalidPeStructure(format!(
            "section '{name}' ended early: expected {len} bytes, copied {copied}"
        )));
    }

    Ok(())
}

pub(crate) fn write_gap<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    let mut remaining = size;
    while remaining > 0 {
        let Ok(chunk) = usize::try_from(remaining.min(8192_u64)) else {
            return Err(YukiError::InvalidPeStructure(
                "gap range overflow".to_owned(),
            ));
        };
        let Some(pad) = ZERO_BUF.get(..chunk) else {
            return Err(YukiError::InvalidPeStructure(
                "zero buffer range invalid".to_owned(),
            ));
        };
        writer.write_all(pad)?;
        let Ok(chunk_u64) = u64::try_from(chunk) else {
            return Err(YukiError::InvalidPeStructure(
                "gap count overflow".to_owned(),
            ));
        };
        remaining = remaining.saturating_sub(chunk_u64);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ErrorWriter;

    impl Write for ErrorWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_gap_writes_zeros() {
        // ARRANGE
        let mut output = Vec::new();

        // ACT
        write_gap(&mut output, 32).unwrap();

        // ASSERT
        assert_eq!(output, vec![0_u8; 32]);
    }

    #[test]
    fn write_gap_propagates_writer_error() {
        // ARRANGE & ACT
        let result = write_gap(&mut ErrorWriter, 100);

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn copy_exact_rejects_short_stream() {
        // ARRANGE
        let data = b"short data";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();

        // ACT
        let result = copy_exact(&mut reader, &mut output, 32, ".test", &mut |_| {});

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("ended early")
        ));
    }

    #[test]
    fn copy_exact_propagates_writer_error() {
        // ARRANGE
        let mut writer = ErrorWriter;

        // ACT
        let result = copy_exact(&mut &b"data"[..], &mut writer, 4, ".test", &mut |_| {});

        // ASSERT
        assert!(matches!(result, Err(YukiError::Io(_))));
    }

    #[test]
    fn copy_exact_invokes_callback() {
        // ARRANGE
        let data = b"hello world";
        let mut reader: &[u8] = data;
        let mut output = Vec::new();
        let mut seen = Vec::new();

        // ACT
        copy_exact(
            &mut reader,
            &mut output,
            u64::try_from(data.len()).unwrap(),
            ".test",
            &mut |chunk| seen.extend_from_slice(chunk),
        )
        .unwrap();

        // ASSERT
        assert_eq!(seen, data);
        assert_eq!(output, data);
    }
}
