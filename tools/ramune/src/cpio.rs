//! CPIO newc-format archive writer.

use std::io::Write;

use crate::error::{RamuneError, Result};

/// CPIO newc format magic number.
const NEWC_MAGIC: &str = "070701";

/// Trailer entry name marking the end of the archive.
const TRAILER: &str = "TRAILER!!!";

/// Zero bytes used for archive padding.
const PAD_SLICES: [&[u8]; 4] = [&[], &[0_u8], &[0_u8, 0_u8], &[0_u8, 0_u8, 0_u8]];

/// A single entry in a CPIO archive.
pub(crate) struct CpioEntry<'a> {
    pub path: &'a str,
    pub mode: u32,
    pub data: &'a [u8],
}

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

/// Writes a CPIO archive containing the given entries into `writer`.
pub(crate) fn write_archive<W: Write>(writer: &mut W, entries: &[CpioEntry<'_>]) -> Result<()> {
    let mut inode = 1_u32;

    for entry in entries {
        write_entry(writer, inode, entry.path, entry.mode, entry.data)?;
        inode = inode
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    write_entry(writer, inode, TRAILER, 0, &[])
}

/// Writes a single entry (file or directory) to the CPIO archive.
fn write_entry<W: Write>(
    writer: &mut W,
    ino: u32,
    name: &str,
    mode: u32,
    data: &[u8],
) -> Result<()> {
    let name_bytes = name.as_bytes();
    let namesize = usize_to_u32(name_bytes.len().saturating_add(1), "filename length")?;
    let filesize = usize_to_u32(data.len(), "file size")?;

    let mut position = write_header(
        writer,
        &CpioHeader {
            ino,
            mode,
            nlink: 1,
            filesize,
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

    if !data.is_empty() {
        writer
            .write_all(data)
            .map_err(|e| RamuneError::CpioError(format!("Failed to write file data: {e}")))?;
        position = position.saturating_add(data.len());
        let _padding = write_pad4(writer, position)?;
    }

    Ok(())
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
        let padding = PAD_SLICES.get(pad).copied().unwrap_or(&[]);
        writer
            .write_all(padding)
            .map_err(|e| RamuneError::CpioError(format!("Failed to write padding: {e}")))?;
    }
    Ok(pad)
}

fn usize_to_u32(value: usize, context: &str) -> Result<u32> {
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

    fn archive_from_entries(entries: &[CpioEntry<'_>]) -> Vec<u8> {
        let mut writer = Vec::new();
        write_archive(&mut writer, entries).expect("write_archive");
        writer
    }

    fn extension_entries<'a>(files: &'a [(&'a str, &'a [u8])]) -> Vec<CpioEntry<'a>> {
        let mut entries = Vec::with_capacity(files.len().saturating_add(1));
        entries.push(CpioEntry {
            path: "extensions",
            mode: 0o040_755,
            data: &[],
        });
        entries.extend(files.iter().map(|&(path, data)| CpioEntry {
            path,
            mode: 0o100_644,
            data,
        }));
        entries
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

    #[test]
    fn failing_writer_flush_succeeds() {
        use std::io::Write as _;

        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        // ACT / ASSERT
        writer.flush().expect("flush");
    }

    #[test]
    fn create_archive_single_file() {
        // ARRANGE
        let entries = extension_entries(&[("extensions/test.txt", b"hello world")]);

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &entries).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
        assert!(
            result
                .windows("test.txt".len())
                .any(|w| w == "test.txt".as_bytes())
        );
    }

    #[test]
    fn create_archive_multiple_files() {
        // ARRANGE
        let entries = extension_entries(&[
            ("extensions/file1.txt", b"content1"),
            ("extensions/file2.txt", b"content2"),
        ]);

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &entries).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
        assert!(
            result
                .windows("file1.txt".len())
                .any(|w| w == "file1.txt".as_bytes())
        );
        assert!(
            result
                .windows("file2.txt".len())
                .any(|w| w == "file2.txt".as_bytes())
        );
    }

    #[test]
    fn create_archive_empty_files() {
        // ARRANGE
        let entries = extension_entries(&[]);

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &entries).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_empty_data() {
        // ARRANGE
        let entries = extension_entries(&[("extensions/empty.txt", b"")]);

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &entries).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_large_data() {
        // ARRANGE
        let large_data = vec![0_u8; 10_000];
        let mut entries = extension_entries(&[]);
        entries.push(CpioEntry {
            path: "extensions/large.bin",
            mode: 0o100_644,
            data: large_data.as_slice(),
        });

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &entries).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(result.len() > 10000);
    }

    #[test]
    fn create_from_entries_with_directories() {
        // ARRANGE
        let entries = vec![
            CpioEntry {
                path: "lib",
                mode: 0o040_755,
                data: &[],
            },
            CpioEntry {
                path: "lib/modules",
                mode: 0o040_755,
                data: &[],
            },
            CpioEntry {
                path: "lib/modules/test.ko",
                mode: 0o100_644,
                data: b"module data",
            },
        ];

        // ACT
        let result = archive_from_entries(&entries);

        // ASSERT
        assert!(!result.is_empty());
        assert!(
            result
                .windows("lib/modules/test.ko".len())
                .any(|w| w == "lib/modules/test.ko".as_bytes())
        );
    }

    #[test]
    fn create_from_entries_empty() {
        // ARRANGE / ACT
        let result = archive_from_entries(&[]);

        // ASSERT
        assert!(
            result
                .windows(TRAILER.len())
                .any(|w| w == TRAILER.as_bytes())
        );
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
    fn write_entries_write_error_propagates() {
        // ARRANGE
        let entry = CpioEntry {
            path: "init",
            mode: 0o100_755,
            data: b"data",
        };
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_archive(&mut writer, &[entry]);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(_)))
        );
    }

    #[test]
    fn write_archive_writes_trailer_on_success() {
        // ARRANGE
        let entries = [CpioEntry {
            path: "init",
            mode: 0o100_755,
            data: b"data",
        }];
        let mut writer = Vec::new();

        // ACT
        write_archive(&mut writer, &entries).expect("write_archive");

        // ASSERT
        assert!(
            writer
                .windows(TRAILER.len())
                .any(|window| window == TRAILER.as_bytes())
        );
    }

    #[test]
    fn create_from_entries_contains_trailer() {
        // ARRANGE
        let entry = CpioEntry {
            path: "init",
            mode: 0o100_755,
            data: b"data",
        };

        // ACT
        let archive = archive_from_entries(&[entry]);

        // ASSERT
        assert!(
            archive
                .windows(TRAILER.len())
                .any(|window| window == TRAILER.as_bytes())
        );
    }

    #[test]
    fn write_archive_maps_writer_errors() {
        // ARRANGE
        let entries = extension_entries(&[("extensions/test.erofs", b"abc")]);
        let cases = [
            (1, "Failed to write header"),
            (2, "Failed to write filename"),
            (3, "Failed to write null byte"),
            (4, "Failed to write padding"),
            (8, "Failed to write file data"),
            (9, "Failed to write padding"),
        ];

        for (fail_on_call, expected_message) in cases {
            let mut writer = FailingWriter {
                fail_on_call,
                calls: 0,
            };

            // ACT
            let result = write_archive(&mut writer, &entries);

            // ASSERT
            let message = result.expect_err("expected CPIO error").to_string();
            assert!(
                message.contains(expected_message),
                "unexpected message: {message}"
            );
        }
    }

    #[test]
    fn write_entry_maps_post_data_padding_errors() {
        // ARRANGE
        let mut writer = FailingWriter {
            fail_on_call: 6,
            calls: 0,
        };

        // ACT
        let result = write_entry(&mut writer, 1, "init", 0o100_755, b"abc");

        // ASSERT
        let message = result
            .expect_err("expected post-data padding error")
            .to_string();
        assert!(message.contains("Failed to write padding"));
    }
}
