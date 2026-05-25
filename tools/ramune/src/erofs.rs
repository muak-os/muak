//! EROFS image creation from a source directory.

use std::path::Path;

use erofs::{Compression, FileContexts, MkfsConfig};

use crate::error::{RamuneError, Result};

/// Creates a reproducible EROFS image with optional `SELinux` file contexts.
pub(crate) fn create(
    source_dir: &Path,
    file_contexts: Option<&FileContexts>,
    compression_level: i32,
) -> Result<Vec<u8>> {
    let compression_level = crate::validate_compression_level(compression_level)?;
    let config = MkfsConfig {
        source_date_epoch: 0,
        file_contexts,
        uuid: [0; 16],
        force_uid: Some(0),
        force_gid: Some(0),
        compression: Compression::Zstd {
            level: compression_level,
        },
    };
    erofs::mkfs(source_dir, &config).map_err(|e| RamuneError::ErofsError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn create_empty_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        // ACT
        let image = create(dir.path(), None, 3).expect("create");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
    }

    #[test]
    fn create_with_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let mut f = NamedTempFile::new_in(dir.path()).expect("tempfile");
        f.write_all(b"hello world").expect("write");

        // ACT
        let image = create(dir.path(), None, 3).expect("create");

        // ASSERT
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_with_subdir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        std::fs::write(dir.path().join("sub").join("file.txt"), b"data").expect("write");

        // ACT
        let image = create(dir.path(), None, 3).expect("create");

        // ASSERT
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_reproducible() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write");

        // ACT
        let img1 = create(dir.path(), None, 3).expect("create 1");
        let img2 = create(dir.path(), None, 3).expect("create 2");

        // ASSERT
        assert_eq!(img1, img2);
    }

    #[test]
    fn create_with_file_contexts() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let fc = FileContexts::from_reader("/.*    system_u:object_r:file_t:s0\n".as_bytes())
            .expect("fc");

        // ACT
        let image = create(dir.path(), Some(&fc), 3).expect("create");

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
    }

    #[test]
    fn create_missing_source_dir_errors() {
        // ARRANGE / ACT
        let result = create(Path::new("/nonexistent/erofs-source"), None, 3);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ErofsError(_)))
        );
    }

    #[test]
    fn create_invalid_compression_level_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        // ACT
        let result = create(dir.path(), None, i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }
}
