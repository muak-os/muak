use std::io::Write;

use crate::error::{ImagerError, Result};

/// CPIO newc format magic number.
const NEWC_MAGIC: &str = "070701";

/// Trailer entry name marking the end of the archive.
const TRAILER: &str = "TRAILER!!!";

/// Writes a CPIO newc format header to the output buffer.
fn write_header(
    writer: &mut Vec<u8>,
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
) -> Result<()> {
    write!(writer, "{NEWC_MAGIC}{ino:08x}{mode:08x}{uid:08x}{gid:08x}{nlink:08x}{mtime:08x}{filesize:08x}{devmajor:08x}{devminor:08x}{rdevmajor:08x}{rdevminor:08x}{namesize:08x}{check:08x}")
        .map_err(|e| ImagerError::CpioError(format!("Failed to write header: {e}")))?;
    Ok(())
}

/// Writes a single entry (file or directory) to the CPIO archive.
fn write_entry(writer: &mut Vec<u8>, ino: u32, name: &str, mode: u32, data: &[u8]) -> Result<()> {
    let name_bytes = name.as_bytes();
    let namesize = (name_bytes.len() + 1) as u32;
    let filesize = data.len() as u32;

    write_header(
        writer, ino, mode, 0, 0, 1, 0, filesize, 0, 0, 0, 0, namesize, 0,
    )?;

    writer
        .write_all(name_bytes)
        .map_err(|e| ImagerError::CpioError(format!("Failed to write filename: {e}")))?;
    writer.push(0);
    pad_to_4(writer);

    if !data.is_empty() {
        writer
            .write_all(data)
            .map_err(|e| ImagerError::CpioError(format!("Failed to write file data: {e}")))?;
        pad_to_4(writer);
    }

    Ok(())
}

/// Pads the output to a 4-byte boundary with null bytes.
fn pad_to_4(writer: &mut Vec<u8>) {
    let len = writer.len();
    let pad = (4 - (len % 4)) % 4;
    writer.extend(std::iter::repeat(0).take(pad));
}

/// Creates a CPIO archive in "newc" format containing the given files.
///
/// Creates an archive with:
/// - An "extensions" directory entry
/// - All files under their specified paths
/// - A proper TRAILER!!! entry
///
/// File mode bits:
/// - Directories: 0o040755 (S_IFDIR | 0755)
/// - Regular files: 0o100644 (S_IFREG | 0644)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_archive_single_file() {
        let files = vec![("test.txt".to_string(), b"hello world".to_vec())];
        let result = create_archive(&files).unwrap();
        assert!(!result.is_empty());
        assert!(
            result
                .windows("test.txt".len())
                .any(|w| w == "test.txt".as_bytes())
        );
    }

    #[test]
    fn test_create_archive_multiple_files() {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"content2".to_vec()),
        ];
        let result = create_archive(&files).unwrap();
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
    fn test_create_archive_empty_files() {
        let files: Vec<(String, Vec<u8>)> = vec![];
        let result = create_archive(&files).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_create_archive_empty_data() {
        let files = vec![("empty.txt".to_string(), vec![])];
        let result = create_archive(&files).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_create_archive_large_data() {
        let large_data = vec![0u8; 10000];
        let files = vec![("large.bin".to_string(), large_data)];
        let result = create_archive(&files).unwrap();
        assert!(result.len() > 10000);
    }
}
