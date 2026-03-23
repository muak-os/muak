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

/// Creates a CPIO archive in newc format containing the given files under an `extensions/` directory.
pub(crate) fn create_archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cpio_data = Vec::new();
    let mut ino = 1u32;

    write_entry(&mut cpio_data, ino, "extensions", 0o040755, &[])?;
    ino += 1;

    for (path, data) in files {
        write_entry(&mut cpio_data, ino, path, 0o100644, data)?;
        ino += 1;
    }

    write_entry(&mut cpio_data, ino, TRAILER, 0, &[])?;

    Ok(cpio_data)
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
}
