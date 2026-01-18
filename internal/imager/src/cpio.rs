use cpio::{NewcBuilder, newc::ModeFileType};
use std::io::Write;

use crate::error::{ImagerError, Result};

pub(crate) fn create_archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cpio_data = Vec::new();
    let mut inode = 1u32;

    let dir_builder = NewcBuilder::new("extensions")
        .ino(inode)
        .uid(0)
        .gid(0)
        .mode(0o755)
        .set_mode_file_type(ModeFileType::Directory);
    inode += 1;
    let writer = dir_builder.write(&mut cpio_data, 0);
    writer
        .finish()
        .map_err(|e| ImagerError::CpioError(format!("Failed to write cpio directory: {}", e)))?;

    for (path, data) in files {
        let builder = NewcBuilder::new(path)
            .ino(inode)
            .uid(0)
            .gid(0)
            .mode(0o644)
            .set_mode_file_type(ModeFileType::Regular);
        inode += 1;

        let mut writer = builder.write(&mut cpio_data, data.len() as u32);
        writer.write_all(data).map_err(|e| {
            ImagerError::CpioError(format!("Failed to write cpio file data: {}", e))
        })?;
        writer
            .finish()
            .map_err(|e| ImagerError::CpioError(format!("Failed to finish cpio entry: {}", e)))?;
    }

    cpio::newc::trailer(&mut cpio_data)
        .map_err(|e| ImagerError::CpioError(format!("Failed to write cpio trailer: {}", e)))?;

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
        assert!(result.len() > 10000); // CPIO overhead
    }
}
