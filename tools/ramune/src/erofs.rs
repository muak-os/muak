//! EROFS image creation from a source directory.

use std::io::Read;
use std::path::Path;

use erofs::dir::EROFS_FT_REG_FILE;
use erofs::layout::{self, ImagePlan};
use erofs::source;
use erofs::tree::TreeEntry;
use erofs::{Compression, FileContexts, MkfsConfig};

use crate::compress;
use crate::error::{RamuneError, Result};

pub(crate) fn plan_image<'a>(
    source_dir: &Path,
    file_contexts: Option<&'a FileContexts>,
    compression_level: i32,
) -> Result<(ImagePlan, MkfsConfig<'a>, u64)> {
    let compression_level = compress::validate_level(compression_level)?;
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

    let entries =
        source::collect_entries(source_dir).map_err(|e| RamuneError::ErofsError(e.to_string()))?;
    let mut readers = build_readers(source_dir, &entries)?;
    let mut files: Vec<erofs::SizedFile<'_>> = entries
        .into_iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| erofs::SizedFile { entry, reader })
        .collect();

    let plan =
        layout::plan(&mut files, &config).map_err(|e| RamuneError::ErofsError(e.to_string()))?;
    let total_size = u64::try_from(plan.total_size).unwrap_or(u64::MAX);

    Ok((plan, config, total_size))
}

enum EntryReader {
    File(std::fs::File),
    Empty(std::io::Empty),
}

impl Read for EntryReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {
            Self::File(ref mut file) => file.read(buf),
            Self::Empty(ref mut empty) => empty.read(buf),
        }
    }
}

fn build_readers(dir: &Path, entries: &[TreeEntry]) -> Result<Vec<EntryReader>> {
    entries
        .iter()
        .map(|ent| {
            if ent.file_type == EROFS_FT_REG_FILE && ent.size > 0 {
                let path = dir.join(ent.rel_path.strip_prefix('/').unwrap_or(&ent.rel_path));
                match std::fs::File::open(&path) {
                    Ok(file) => Ok(EntryReader::File(file)),
                    Err(source) => Err(RamuneError::ReadError {
                        file: path.display().to_string(),
                        source,
                    }),
                }
            } else {
                Ok(EntryReader::Empty(std::io::empty()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use erofs::writer;
    use tempfile::NamedTempFile;

    use super::*;

    fn run_create(dir: &Path, fc: Option<&FileContexts>, clevel: i32) -> Vec<u8> {
        let (plan, config, _size) = plan_image(dir, fc, clevel).expect("plan_image");
        let mut buf = Vec::new();
        writer::image(&mut buf, &plan, &config).expect("image");

        buf
    }

    #[test]
    fn create_empty_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        // ACT
        let image = run_create(dir.path(), None, 3);

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len().rem_euclid(4096), 0);
    }

    #[test]
    fn create_with_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let mut file = NamedTempFile::new_in(dir.path()).expect("tempfile");
        file.write_all(b"hello world").expect("write");

        // ACT
        let image = run_create(dir.path(), None, 3);

        // ASSERT
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_with_subdir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir");
        std::fs::write(dir.path().join("sub").join("file.txt"), b"data").expect("write");

        // ACT
        let image = run_create(dir.path(), None, 3);

        // ASSERT
        assert!(image.len() >= 4096);
    }

    #[test]
    fn create_reproducible() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"aaa").expect("write");

        // ACT
        let img1 = run_create(dir.path(), None, 3);
        let img2 = run_create(dir.path(), None, 3);

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
        let image = run_create(dir.path(), Some(&fc), 3);

        // ASSERT
        assert!(!image.is_empty());
        assert_eq!(image.len().rem_euclid(4096), 0);
    }

    #[test]
    fn create_missing_source_dir_errors() {
        // ARRANGE / ACT
        let result = plan_image(Path::new("/nonexistent/erofs-source"), None, 3);

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
        let result = plan_image(dir.path(), None, i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }
}
