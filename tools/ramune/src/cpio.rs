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
pub(crate) fn create_from_entries(entries: &[CpioEntry]) -> Result<Vec<u8>> {
    let capacity = entries
        .iter()
        .map(|e| 110 + e.path.len() + e.data.len() + 8)
        .sum();
    let mut buf = Vec::with_capacity(capacity);
    write_entries(&mut buf, entries)?;
    Ok(buf)
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
fn write_entries<W: Write>(writer: &mut W, entries: &[CpioEntry]) -> Result<()> {
    for (i, entry) in entries.iter().enumerate() {
        write_entry(writer, (i + 1) as u32, &entry.path, entry.mode, &entry.data)?;
    }
    write_entry(writer, (entries.len() + 1) as u32, TRAILER, 0, &[])
}

/// Writes a CPIO newc format header to `writer`, returning bytes written.
fn write_header<W: Write>(writer: &mut W, h: &CpioHeader) -> Result<usize> {
    let s = format!(
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
    );
    writer
        .write_all(s.as_bytes())
        .map_err(|e| RamuneError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(s.len())
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let result = create_from_entries(&entries).unwrap();

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
        let result = create_from_entries(&[]).unwrap();

        // ASSERT
        assert!(
            result
                .windows(TRAILER.len())
                .any(|w| w == TRAILER.as_bytes())
        );
    }
}
