//! CPIO newc-format archive writer.

use std::io::Write;

use crate::error::{RamuneError, Result};

/// CPIO newc format magic number.
const NEWC_MAGIC: &str = "070701";

/// Trailer entry name marking the end of the archive.
const TRAILER: &str = "TRAILER!!!";

/// A single entry in a CPIO archive.
pub(crate) struct CpioEntry {
    pub path: String,
    pub mode: u32,
    pub data: Vec<u8>,
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

/// Creates a CPIO archive in newc format from a list of entries.
pub(crate) fn create_from_entries(entries: &[CpioEntry]) -> Vec<u8> {
    let capacity = entries
        .iter()
        .map(|e| 110 + e.path.len() + e.data.len() + 8)
        .sum();
    let mut buf = Vec::with_capacity(capacity);
    write_entries_to_vec(&mut buf, entries);
    buf
}

/// Writes a CPIO archive containing the given files under an `extensions/` directory into `writer`.
pub(crate) fn write_archive<W: Write>(writer: &mut W, files: &[(String, Vec<u8>)]) -> Result<()> {
    let dir_entry = CpioEntry {
        path: "extensions".to_string(),
        mode: 0o040755,
        data: Vec::new(),
    };
    write_entry(writer, 1, &dir_entry.path, dir_entry.mode, &dir_entry.data)?;
    for (ino, (path, data)) in files.iter().enumerate() {
        write_entry(writer, (ino + 2) as u32, path, 0o100644, data)?;
    }
    write_entry(writer, (files.len() + 2) as u32, TRAILER, 0, &[])
}

/// Writes all entries plus the TRAILER to `writer`.
#[cfg(test)]
fn write_entries<W: Write>(writer: &mut W, entries: &[CpioEntry]) -> Result<()> {
    for (i, entry) in entries.iter().enumerate() {
        write_entry(writer, (i + 1) as u32, &entry.path, entry.mode, &entry.data)?;
    }
    write_entry(writer, (entries.len() + 1) as u32, TRAILER, 0, &[])
}

fn write_entries_to_vec(buf: &mut Vec<u8>, entries: &[CpioEntry]) {
    for (i, entry) in entries.iter().enumerate() {
        write_entry_to_vec(buf, (i + 1) as u32, &entry.path, entry.mode, &entry.data);
    }
    write_entry_to_vec(buf, (entries.len() + 1) as u32, TRAILER, 0, &[]);
}

fn header_string(h: &CpioHeader) -> String {
    format!(
        "{NEWC_MAGIC}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h.ino,
        h.mode,
        h.uid,
        h.gid,
        h.nlink,
        h.mtime,
        h.filesize,
        h.devmajor,
        h.devminor,
        h.rdevmajor,
        h.rdevminor,
        h.namesize,
        h.check,
    )
}

/// Writes a CPIO newc format header to `writer`, returning bytes written.
fn write_header<W: Write>(writer: &mut W, h: &CpioHeader) -> Result<usize> {
    let s = header_string(h);
    writer
        .write_all(s.as_bytes())
        .map_err(|e| RamuneError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(s.len())
}

fn write_header_to_vec(buf: &mut Vec<u8>, h: &CpioHeader) -> usize {
    let s = header_string(h);
    buf.extend_from_slice(s.as_bytes());
    s.len()
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
    let namesize = (name_bytes.len() + 1) as u32;
    let filesize = data.len() as u32;

    let mut pos = write_header(
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
    pos += name_bytes.len() + 1;
    pos += write_pad4(writer, pos)?;

    if !data.is_empty() {
        writer
            .write_all(data)
            .map_err(|e| RamuneError::CpioError(format!("Failed to write file data: {e}")))?;
        pos += data.len();
        write_pad4(writer, pos)?;
    }

    Ok(())
}

fn write_entry_to_vec(buf: &mut Vec<u8>, ino: u32, name: &str, mode: u32, data: &[u8]) {
    let name_bytes = name.as_bytes();
    let namesize = (name_bytes.len() + 1) as u32;
    let filesize = data.len() as u32;

    let mut pos = write_header_to_vec(
        buf,
        &CpioHeader {
            ino,
            mode,
            nlink: 1,
            filesize,
            namesize,
            ..CpioHeader::default()
        },
    );

    buf.extend_from_slice(name_bytes);
    buf.push(0);
    pos += name_bytes.len() + 1;
    pos += write_pad4_to_vec(buf, pos);

    if !data.is_empty() {
        buf.extend_from_slice(data);
        pos += data.len();
        write_pad4_to_vec(buf, pos);
    }
}

/// Writes null padding to align `pos` to a 4-byte boundary; returns bytes written.
fn write_pad4<W: Write>(writer: &mut W, pos: usize) -> Result<usize> {
    let pad = (4 - (pos % 4)) % 4;
    if pad > 0 {
        writer
            .write_all(&[0u8; 3][..pad])
            .map_err(|e| RamuneError::CpioError(format!("Failed to write padding: {e}")))?;
    }
    Ok(pad)
}

fn write_pad4_to_vec(buf: &mut Vec<u8>, pos: usize) -> usize {
    let pad = (4 - (pos % 4)) % 4;
    buf.resize(buf.len() + pad, 0);
    pad
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    struct FailingWriter {
        fail_on_call: usize,
        calls: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            let should_fail = self.calls == self.fail_on_call;

            match should_fail {
                true => Err(std::io::Error::other("boom")),
                false => Ok(buf.len()),
            }
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
        let files = vec![("test.txt".to_string(), b"hello world".to_vec())];

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &files).expect("write_archive");
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
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"content2".to_vec()),
        ];

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &files).expect("write_archive");
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
        let files: Vec<(String, Vec<u8>)> = vec![];

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &files).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_empty_data() {
        // ARRANGE
        let files = vec![("empty.txt".to_string(), vec![])];

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &files).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_large_data() {
        // ARRANGE
        let large_data = vec![0u8; 10000];
        let files = vec![("large.bin".to_string(), large_data)];

        // ACT
        let mut buf = Vec::new();
        write_archive(&mut buf, &files).expect("write_archive");
        let result = buf;

        // ASSERT
        assert!(result.len() > 10000);
    }

    #[test]
    fn create_from_entries_with_directories() {
        // ARRANGE
        let entries = vec![
            CpioEntry {
                path: "lib".to_string(),
                mode: 0o040755,
                data: Vec::new(),
            },
            CpioEntry {
                path: "lib/modules".to_string(),
                mode: 0o040755,
                data: Vec::new(),
            },
            CpioEntry {
                path: "lib/modules/test.ko".to_string(),
                mode: 0o100644,
                data: b"module data".to_vec(),
            },
        ];

        // ACT
        let result = create_from_entries(&entries);

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
        let result = create_from_entries(&[]);

        // ASSERT
        assert!(
            result
                .windows(TRAILER.len())
                .any(|w| w == TRAILER.as_bytes())
        );
    }

    #[test]
    fn write_entries_write_error_propagates() {
        // ARRANGE
        let entry = CpioEntry {
            path: "init".to_string(),
            mode: 0o100755,
            data: b"data".to_vec(),
        };
        let mut writer = FailingWriter {
            fail_on_call: 1,
            calls: 0,
        };

        // ACT
        let result = write_entries(&mut writer, &[entry]);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(_)))
        );
    }

    #[test]
    fn write_entries_writes_trailer_on_success() {
        // ARRANGE
        let entries = [CpioEntry {
            path: "init".to_string(),
            mode: 0o100755,
            data: b"data".to_vec(),
        }];
        let mut writer = Vec::new();

        // ACT
        write_entries(&mut writer, &entries).expect("write_entries");

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
            path: "init".to_string(),
            mode: 0o100755,
            data: b"data".to_vec(),
        };

        // ACT
        let archive = create_from_entries(&[entry]);

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
        let files = vec![("extensions/test.erofs".to_string(), b"abc".to_vec())];
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
            let result = write_archive(&mut writer, &files);

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
        let result = write_entry(&mut writer, 1, "init", 0o100755, b"abc");

        // ASSERT
        let message = result
            .expect_err("expected post-data padding error")
            .to_string();
        assert!(message.contains("Failed to write padding"));
    }
}
