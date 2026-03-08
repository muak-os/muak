use std::io::Cursor;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use walkdir::WalkDir;

use crate::error::{ImagerError, Result};

// Constants for squashfs configuration
const BLOCK_SIZE: u32 = 1024 * 1024;
const ROOT_UID: u32 = 0;
const ROOT_GID: u32 = 0;
const SYMLINK_MODE: u16 = 0o777;

/// Get the file mode from metadata, masked to permissions only
fn get_mode(metadata: &std::fs::Metadata) -> u16 {
    (metadata.mode() & 0o7777) as u16
}

/// Get the modification time from metadata
fn get_mtime(metadata: &std::fs::Metadata) -> u32 {
    metadata.mtime() as u32
}

/// Set up the squashfs writer with default configuration
fn setup_writer(source_dir: &Path) -> Result<FilesystemWriter<'_, '_, '_>> {
    let mut writer = FilesystemWriter::default();
    let compressor = FilesystemCompressor::new(Compressor::Zstd, None)
        .map_err(|e| ImagerError::SquashfsError(format!("Failed to create compressor: {}", e)))?;
    writer.set_compressor(compressor);
    writer.set_block_size(BLOCK_SIZE);

    let root_metadata = std::fs::metadata(source_dir).map_err(|e| ImagerError::ReadError {
        file: source_dir.display().to_string(),
        source: e,
    })?;
    let root_mode = get_mode(&root_metadata);
    writer.set_root_mode(root_mode);
    writer.set_root_uid(ROOT_UID);
    writer.set_root_gid(ROOT_GID);

    Ok(writer)
}

/// Add a single entry to the squashfs writer
fn add_entry_to_writer(writer: &mut FilesystemWriter, path: &Path, rel_path: &Path) -> Result<()> {
    let archive_path = format!("/{}", rel_path.display());
    let metadata = path
        .metadata()
        .map_err(|e| ImagerError::SquashfsError(format!("Failed to read metadata: {}", e)))?;

    let mode = get_mode(&metadata);
    let mtime = get_mtime(&metadata);

    if metadata.is_dir() {
        let header = NodeHeader::new(mode, 0, 0, mtime);
        writer
            .push_dir(&archive_path, header)
            .map_err(|e| ImagerError::SquashfsError(format!("Failed to add directory: {}", e)))?;
    } else if metadata.is_symlink() {
        let link_target = std::fs::read_link(path)
            .map_err(|e| ImagerError::SquashfsError(format!("Failed to read symlink: {}", e)))?;
        let header = NodeHeader::new(SYMLINK_MODE, 0, 0, mtime);
        writer
            .push_symlink(
                link_target.to_string_lossy().into_owned(),
                &archive_path,
                header,
            )
            .map_err(|e| ImagerError::SquashfsError(format!("Failed to add symlink: {}", e)))?;
    } else if metadata.is_file() {
        let contents = std::fs::read(path).map_err(|e| ImagerError::ReadError {
            file: path.display().to_string(),
            source: e,
        })?;
        let header = NodeHeader::new(mode, 0, 0, mtime);
        writer
            .push_file(Cursor::new(contents), &archive_path, header)
            .map_err(|e| ImagerError::SquashfsError(format!("Failed to add file: {}", e)))?;
    }

    Ok(())
}

pub(crate) fn create_at(source_dir: &Path) -> Result<Vec<u8>> {
    let mut writer = setup_writer(source_dir)?;

    for entry in WalkDir::new(source_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry
            .map_err(|e| ImagerError::SquashfsError(format!("Failed to walk directory: {}", e)))?;
        let path = entry.path();
        let rel_path = path.strip_prefix(source_dir).map_err(|e| {
            ImagerError::SquashfsError(format!("Failed to strip path prefix: {}", e))
        })?;

        if rel_path.as_os_str().is_empty() {
            continue;
        }

        add_entry_to_writer(&mut writer, path, rel_path)?;
    }

    let mut output = Cursor::new(Vec::new());
    writer
        .write(&mut output)
        .map_err(|e| ImagerError::SquashfsError(format!("Failed to write squashfs: {}", e)))?;
    Ok(output.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn test_get_mode_regular_file() {
        // ARRANGE
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test").unwrap();
        let metadata = temp_file.as_file().metadata().unwrap();

        // ACT
        let mode = get_mode(&metadata);

        // ASSERT
        assert!(mode & 0o400 != 0);
    }

    #[test]
    fn test_get_mode_directory() {
        // ARRANGE
        let temp_dir = tempfile::tempdir().unwrap();
        let metadata = temp_dir.path().metadata().unwrap();

        // ACT
        let mode = get_mode(&metadata);

        // ASSERT
        assert!(mode & 0o400 != 0);
    }

    #[test]
    fn test_get_mtime() {
        // ARRANGE
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test").unwrap();
        let metadata = temp_file.as_file().metadata().unwrap();

        // ACT
        let mtime = get_mtime(&metadata);

        // ASSERT
        assert!(mtime > 0);
    }

    #[test]
    fn test_get_mode_custom_permissions() {
        // ARRANGE
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test").unwrap();
        let mut perms = temp_file.as_file().metadata().unwrap().permissions();
        perms.set_mode(0o755);
        temp_file.as_file().set_permissions(perms).unwrap();
        let metadata = temp_file.as_file().metadata().unwrap();

        // ACT
        let mode = get_mode(&metadata);

        // ASSERT
        assert_eq!(mode & 0o777, 0o755);
    }
}
