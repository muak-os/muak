//! EROFS image creation from a source directory.

use std::path::Path;

use erofs::MkfsConfig;

use crate::error::{ImagerError, Result};

pub(crate) fn create_at(source_dir: &Path) -> Result<Vec<u8>> {
    let config = MkfsConfig {
        source_date_epoch: 0,
        file_contexts: None,
        uuid: [0; 16],
        force_uid: Some(0),
        force_gid: Some(0),
        compress: true,
    };
    erofs::mkfs(source_dir, &config).map_err(|e| ImagerError::ErofsError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn create_at_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let image = create_at(dir.path()).expect("create_at");
        assert!(!image.is_empty());
        assert_eq!(image.len() % 4096, 0);
    }

    #[test]
    fn create_at_with_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut f = NamedTempFile::new_in(dir.path()).expect("tempfile");
        f.write_all(b"hello world").expect("write");
        let image = create_at(dir.path()).expect("create_at");
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_at_with_subdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        std::fs::write(dir.path().join("sub").join("file.txt"), b"data").expect("write");
        let image = create_at(dir.path()).expect("create_at");
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_at_reproducible() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write");
        let img1 = create_at(dir.path()).expect("create_at 1");
        let img2 = create_at(dir.path()).expect("create_at 2");
        assert_eq!(img1, img2);
    }
}
