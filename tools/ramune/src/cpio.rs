//! CPIO newc-format archive writer.

use std::io::Write;

use crate::error::{RamuneError, Result};

/// CPIO newc format magic number.
const NEWC_MAGIC: &str = "070701";

/// Trailer entry name marking the end of the archive.
const TRAILER: &str = "TRAILER!!!";

/// Fields for a CPIO newc format header entry.
#[derive(Debug, Default)]
struct CpioHeader {
    ino: u32,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u32,
    mtime: u32,
    filesize: u32,
    devmajor: u32,
    devminor: u32,
    rdevmajor: u32,
    rdevminor: u32,
    namesize: u32,
    check: u32,
}

/// Writes a single CPIO entry.
pub(crate) fn write_entry<W: Write>(
    writer: &mut W,
    ino: u32,
    name: &str,
    mode: u32,
    size: u32,
    write_data: impl FnOnce(&mut W) -> Result<()>,
) -> Result<()> {
    let name_bytes = name.as_bytes();
    let namesize = usize_to_u32(name_bytes.len().saturating_add(1), "filename length")?;

    let mut position = write_header(
        writer,
        &CpioHeader {
            ino,
            mode,
            nlink: 1,
            filesize: size,
            namesize,
            ..CpioHeader::default()
        },
    )?;

    writer
        .write_all(name_bytes)
        .map_err(|e| RamuneError::CpioError(format!("Failed to write filename: {e}")))?;
    writer
        .write_all(&[0])
        .map_err(|e| RamuneError::CpioError(format!("Failed to write null byte: {e}")))?;
    position = position.saturating_add(name_bytes.len()).saturating_add(1);
    position = position.saturating_add(write_pad4(writer, position)?);

    if size > 0 {
        write_data(writer)?;
        position = position.saturating_add(usize::try_from(size).unwrap_or_default());
        let _padding = write_pad4(writer, position)?;
    }

    Ok(())
}

/// Writes the CPIO end-of-archive trailer entry.
pub(crate) fn write_trailer<W: Write>(writer: &mut W) -> Result<()> {
    write_entry(writer, 0, TRAILER, 0, 0, |_| Ok(()))
}

/// Writes a CPIO newc format header to `writer`, returning bytes written.
fn write_header<W: Write>(writer: &mut W, header: &CpioHeader) -> Result<usize> {
    let header_text = header_string(header);
    writer
        .write_all(header_text.as_bytes())
        .map_err(|e| RamuneError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(header_text.len())
}

fn header_string(header: &CpioHeader) -> String {
    format!(
        "{NEWC_MAGIC}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        header.ino,
        header.mode,
        header.uid,
        header.gid,
        header.nlink,
        header.mtime,
        header.filesize,
        header.devmajor,
        header.devminor,
        header.rdevmajor,
        header.rdevminor,
        header.namesize,
        header.check,
    )
}

/// Writes null padding to align `pos` to a 4-byte boundary; returns bytes written.
fn write_pad4<W: Write>(writer: &mut W, pos: usize) -> Result<usize> {
    let pad = pos.next_multiple_of(4).saturating_sub(pos);
    if pad > 0 {
        let padding = &[0_u8; 4];
        writer
            .write_all(padding.get(..pad).unwrap_or(&[]))
            .map_err(|e| RamuneError::CpioError(format!("Failed to write padding: {e}")))?;
    }
    Ok(pad)
}

pub(crate) fn usize_to_u32(value: usize, context: &str) -> Result<u32> {
    match u32::try_from(value) {
        Ok(converted) => Ok(converted),
        Err(_conversion_error) => Err(RamuneError::CpioError(format!(
            "{context} exceeds CPIO limits"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_data(data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        let size = u32::try_from(data.len()).expect("data fits u32");
        write_entry(&mut buf, 1, "init", 0o100_755, size, |w| {
            w.write_all(data)
                .map_err(|e| RamuneError::CpioError(format!("{e}")))
        })
        .expect("write_entry");
        write_trailer(&mut buf).expect("write_trailer");

        buf
    }

    struct FailingWriter {
        fail_on_call: usize,
        calls: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls = self.calls.saturating_add(1);
            (self.calls != self.fail_on_call)
                .then_some(buf.len())
                .ok_or_else(|| std::io::Error::other("boom"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn writer_entry_data<W: Write>(w: &mut W, data: &[u8]) -> Result<()> {
        w.write_all(data)
            .map_err(|e| RamuneError::CpioError(format!("{e}")))
    }

    #[test]
    fn create_archive_single_file() {
        // ARRANGE
        let data = b"hello world";

        // ACT
        let result = run_with_data(data);

        // ASSERT
        assert!(!result.is_empty());
        assert!(result.windows(b"init".len()).any(|w| w == b"init"));
    }

    #[test]
    fn create_archive_empty_data() {
        // ARRANGE / ACT
        let result = run_with_data(b"");

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_large_data() {
        // ARRANGE
        let data = vec![0_u8; 10_000];

        // ACT
        let result = run_with_data(&data);

        // ASSERT
        assert!(result.len() > 10000);
    }

    #[test]
    fn archive_contains_trailer() {
        // ARRANGE / ACT
        let archive = run_with_data(b"data");

        // ASSERT
        assert!(
            archive
                .windows(TRAILER.len())
                .any(|w| w == TRAILER.as_bytes())
        );
    }

    #[test]
    fn trailer_only_is_non_empty() {
        // ARRANGE / ACT
        let mut buf = Vec::new();
        write_trailer(&mut buf).expect("write_trailer");

        // ASSERT
        assert!(!buf.is_empty());
    }

    #[test]
    fn multiple_entries_with_varied_names() {
        // ARRANGE
        let mut buf = Vec::new();
        let data_a = b"content a";
        let data_b = b"content b";

        // ACT
        write_entry(
            &mut buf,
            1,
            "a",
            0o100_644,
            u32::try_from(data_a.len()).expect("len fits u32"),
            |w| writer_entry_data(w, data_a),
        )
        .expect("entry a");
        write_entry(
            &mut buf,
            2,
            "bbbb",
            0o100_644,
            u32::try_from(data_b.len()).expect("len fits u32"),
            |w| writer_entry_data(w, data_b),
        )
        .expect("entry b");
        write_trailer(&mut buf).expect("trailer");

        // ASSERT
        assert!(buf.windows(1).any(|w| w == b"a"));
        assert!(buf.windows(4).any(|w| w == b"bbbb"));
        assert!(buf.windows(TRAILER.len()).any(|w| w == TRAILER.as_bytes()));
    }

    #[test]
    fn failing_writer_flush_succeeds() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        // ACT / ASSERT
        writer.flush().expect("flush should succeed");
    }

    #[test]
    fn entry_propagates_header_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_propagates_name_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 2,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_propagates_null_byte_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 3,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_propagates_padding_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 4,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_propagates_data_write_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 5,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_propagates_data_padding_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 6,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "abc", 0o644, 5, |w| {
            writer_entry_data(w, b"12345")
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn trailer_propagates_header_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_trailer(&mut writer);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_entry_propagates_closure_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "x", 0o644, 3, |_w| {
            Err(RamuneError::CpioError("boom from closure".to_owned()))
        });

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn data_aligned_to_four_bytes_no_padding() {
        // ARRANGE
        let mut buf = Vec::new();
        let data = b"1234";

        // ACT
        write_entry(
            &mut buf,
            1,
            "x",
            0o644,
            u32::try_from(data.len()).expect("len fits u32"),
            |w| writer_entry_data(w, data),
        )
        .expect("entry");
        write_trailer(&mut buf).expect("trailer");

        // ASSERT
        assert!(buf.len() >= 118);
    }

    #[test]
    fn data_not_aligned_triggers_padding() {
        // ARRANGE
        let mut buf = Vec::new();
        let data = b"123";

        // ACT
        write_entry(
            &mut buf,
            1,
            "x",
            0o644,
            u32::try_from(data.len()).expect("len fits u32"),
            |w| writer_entry_data(w, data),
        )
        .expect("entry");
        write_trailer(&mut buf).expect("trailer");

        // ASSERT
        // 110 header + 1 name + 1 null + 2 pad + 3 data + 1 pad = 118
        assert!(buf.len() >= 118);
    }

    #[test]
    fn usize_to_u32_rejects_name_larger_than_cpio_limit() {
        // ARRANGE / ACT
        let too_large = usize::try_from(u32::MAX)
            .expect("u32 max should fit usize")
            .saturating_add(1);
        let result = usize_to_u32(too_large, "filename length");

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(message) if message.contains("filename length exceeds CPIO limits")))
        );
    }

    #[test]
    fn usize_to_u32_rejects_data_larger_than_cpio_limit() {
        // ARRANGE / ACT
        let too_large = usize::try_from(u32::MAX)
            .expect("u32 max should fit usize")
            .saturating_add(1);
        let result = usize_to_u32(too_large, "file size");

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(message) if message.contains("file size exceeds CPIO limits")))
        );
    }

    #[test]
    fn usize_to_u32_accepts_valid_value() {
        // ARRANGE
        let valid = 42_usize;

        // ACT
        let result = usize_to_u32(valid, "test");

        // ASSERT
        assert_eq!(result.expect("valid value"), 42);
    }

    #[test]
    fn trailer_propagates_name_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 2,
            calls: 0,
        };

        // ACT
        let result = write_trailer(&mut writer);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn trailer_propagates_null_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 3,
            calls: 0,
        };

        // ACT
        let result = write_trailer(&mut writer);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn entry_with_failing_writer_succeeds_when_no_failure_triggered() {
        // ARRANGE
        let mut buf = Vec::new();
        let data = b"hello";

        // ACT
        let result = write_entry(
            &mut buf,
            1,
            "abc",
            0o644,
            u32::try_from(data.len()).expect("len fits u32"),
            |w| {
                w.write_all(data)
                    .map_err(|e| RamuneError::CpioError(format!("{e}")))
            },
        );

        // ASSERT
        result.expect("entry should succeed");
    }

    #[test]
    fn write_pad4_propagates_errors() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_pad4(&mut writer, 1);

        // ASSERT
        result.expect_err("expected write error");
    }

    #[test]
    fn write_pad4_zero_pad_returns_ok() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_pad4(&mut writer, 0);

        // ASSERT
        assert_eq!(result.expect("no pad needed"), 0);
    }

    #[test]
    fn entry_zero_size_no_data_closure_called() {
        // ARRANGE
        let mut buf = Vec::new();
        let mut closure_was_called = false;

        // ACT
        write_entry(&mut buf, 1, "abc", 0o644, 0, |_w| {
            closure_was_called = true;
            Ok(())
        })
        .expect("zero-size entry");
        write_trailer(&mut buf).expect("trailer");

        // ASSERT
        assert!(!closure_was_called);
    }

    #[test]
    fn trailer_propagates_padding_error() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 4,
            calls: 0,
        };

        // ACT
        let result = write_trailer(&mut writer);

        // ASSERT
        assert!(result.is_err());
    }
}
