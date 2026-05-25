//! CPIO newc-format archive writer.

use std::io::Write;

use crate::error::{RamuneError, Result};

/// CPIO newc format magic number.
const NEWC_MAGIC: &str = "070701";

/// Trailer entry name marking the end of the archive.
const TRAILER: &str = "TRAILER!!!";

/// Fixed width of a `newc` header.
const HEADER_LEN: usize = 110;

/// Zero bytes used for archive padding.
const PAD_SLICES: [&[u8]; 4] = [&[], &[0_u8], &[0_u8, 0_u8], &[0_u8, 0_u8, 0_u8]];

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
pub(crate) fn create_from_entries(entries: &[CpioEntry]) -> Result<Vec<u8>> {
    let capacity = entries.iter().fold(0_usize, |total, entry| {
        total
            .saturating_add(HEADER_LEN)
            .saturating_add(entry.path.len())
            .saturating_add(entry.data.len())
            .saturating_add(8)
    });
    let mut buf = Vec::with_capacity(capacity);
    write_entries_to_vec(&mut buf, entries)?;
    Ok(buf)
}

/// Writes a CPIO archive containing the given files under an `extensions/` directory into `writer`.
pub(crate) fn write_archive<W: Write>(writer: &mut W, files: &[(String, Vec<u8>)]) -> Result<()> {
    let dir_entry = CpioEntry {
        path: "extensions".to_owned(),
        mode: 0o040_755,
        data: Vec::new(),
    };
    write_entry(writer, 1, &dir_entry.path, dir_entry.mode, &dir_entry.data)?;
    let mut inode = 2_u32;

    for file in files {
        write_entry(writer, inode, &file.0, 0o100_644, &file.1)?;
        inode = inode
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    write_entry(writer, inode, TRAILER, 0, &[])
}

fn write_entries_to_vec(buf: &mut Vec<u8>, entries: &[CpioEntry]) -> Result<()> {
    let mut inode = 1_u32;

    for entry in entries {
        write_entry_to_vec(buf, inode, &entry.path, entry.mode, &entry.data)?;
        inode = inode
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    write_entry_to_vec(buf, inode, TRAILER, 0, &[])?;

    Ok(())
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

/// Writes a CPIO newc format header to `writer`, returning bytes written.
fn write_header<W: Write>(writer: &mut W, header: &CpioHeader) -> Result<usize> {
    let header_text = header_string(header);
    writer
        .write_all(header_text.as_bytes())
        .map_err(|e| RamuneError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(header_text.len())
}

fn write_header_to_vec(buf: &mut Vec<u8>, header: &CpioHeader) -> usize {
    let header_text = header_string(header);
    buf.extend_from_slice(header_text.as_bytes());
    header_text.len()
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

fn write_entry_to_vec(
    buf: &mut Vec<u8>,
    ino: u32,
    name: &str,
    mode: u32,
    data: &[u8],
) -> Result<()> {
    let name_bytes = name.as_bytes();
    let namesize = usize_to_u32(name_bytes.len().saturating_add(1), "filename length")?;
    let filesize = usize_to_u32(data.len(), "file size")?;

    let mut position = write_header_to_vec(
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
    position = position.saturating_add(name_bytes.len()).saturating_add(1);
    position = position.saturating_add(write_pad4_to_vec(buf, position));

    if !data.is_empty() {
        buf.extend_from_slice(data);
        position = position.saturating_add(data.len());
        let _padding = write_pad4_to_vec(buf, position);
    }

    Ok(())
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

fn write_pad4_to_vec(buf: &mut Vec<u8>, pos: usize) -> usize {
    let pad = pos.next_multiple_of(4).saturating_sub(pos);
    let new_len = buf.len().saturating_add(pad);
    buf.resize(new_len, 0);
    pad
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

    fn write_entries<W: Write>(writer: &mut W, entries: &[CpioEntry]) -> Result<()> {
        let mut inode = 1_u32;

        for entry in entries {
            write_entry(writer, inode, &entry.path, entry.mode, &entry.data)?;
            inode = inode
                .checked_add(1)
                .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
        }

        write_entry(writer, inode, TRAILER, 0, &[])
    }

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
        let result = create_from_entries(&entries).expect("create_from_entries");

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
        let result = create_from_entries(&[]).expect("create_from_entries");

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
        let archive = create_from_entries(&[entry]).expect("create_from_entries");

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
