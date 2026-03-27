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
    let mut ino = 1u32;

    for entry in entries {
        write_entry(&mut buf, ino, &entry.path, entry.mode, &entry.data)?;
        ino += 1;
    }

    write_entry(&mut buf, ino, TRAILER, 0, &[])?;
    Ok(buf)
}

/// Creates a CPIO archive containing the given files under an `extensions/` directory.
pub(crate) fn create_archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut entries = Vec::with_capacity(files.len() + 1);
    entries.push(CpioEntry {
        path: "extensions".to_string(),
        mode: 0o040755,
        data: Vec::new(),
    });
    for (path, data) in files {
        entries.push(CpioEntry {
            path: path.clone(),
            mode: 0o100644,
            data: data.clone(),
        });
    }
    create_from_entries(&entries)
}

/// Writes a CPIO newc format header to the output buffer.
fn write_header(writer: &mut Vec<u8>, h: &CpioHeader) -> Result<()> {
    write!(
        writer,
        "{NEWC_MAGIC}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h.ino, h.mode, h.uid, h.gid, h.nlink, h.mtime, h.filesize,
        h.devmajor, h.devminor, h.rdevmajor, h.rdevminor, h.namesize, h.check,
    )
    .map_err(|e| RamuneError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(())
}

/// Writes a single entry (file or directory) to the CPIO archive.
fn write_entry(writer: &mut Vec<u8>, ino: u32, name: &str, mode: u32, data: &[u8]) -> Result<()> {
    let name_bytes = name.as_bytes();
    let namesize = (name_bytes.len() + 1) as u32;
    let filesize = data.len() as u32;

    write_header(
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
    writer.push(0);
    pad_to_4(writer);

    if !data.is_empty() {
        writer
            .write_all(data)
            .map_err(|e| RamuneError::CpioError(format!("Failed to write file data: {e}")))?;
        pad_to_4(writer);
    }

    Ok(())
}

/// Pads the output buffer to the next 4-byte boundary with null bytes.
fn pad_to_4(writer: &mut Vec<u8>) {
    let len = writer.len();
    let pad = (4 - (len % 4)) % 4;
    writer.extend(std::iter::repeat_n(0, pad));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_archive_single_file() {
        // ARRANGE
        let files = vec![("test.txt".to_string(), b"hello world".to_vec())];

        // ACT
        let result = create_archive(&files).unwrap();

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
        let result = create_archive(&files).unwrap();

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
        let result = create_archive(&files).unwrap();

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_empty_data() {
        // ARRANGE
        let files = vec![("empty.txt".to_string(), vec![])];

        // ACT
        let result = create_archive(&files).unwrap();

        // ASSERT
        assert!(!result.is_empty());
    }

    #[test]
    fn create_archive_large_data() {
        // ARRANGE
        let large_data = vec![0u8; 10000];
        let files = vec![("large.bin".to_string(), large_data)];

        // ACT
        let result = create_archive(&files).unwrap();

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
