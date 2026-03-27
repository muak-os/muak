//! Concurrent processing of pre-extracted extension directories into named EROFS blobs.

use std::path::PathBuf;

use crate::erofs;
use crate::error::{RamuneError, Result};

/// Maximum number of extensions processed concurrently.
pub(crate) const MAX_CONCURRENT: usize = 8;

/// Processes all extension directories concurrently, returning `(archive_path, erofs_bytes)` pairs.
pub(crate) async fn process_all(extensions: &[PathBuf]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut join_set = tokio::task::JoinSet::new();
    let mut iter = extensions.iter().cloned();
    let mut files = Vec::with_capacity(extensions.len());

    spawn_batch(&mut join_set, &mut iter);

    while let Some(result) = join_set.join_next().await {
        files.push(result.map_err(|e| RamuneError::ErofsError(e.to_string()))??);
        spawn_batch(&mut join_set, &mut iter);
    }

    Ok(files)
}

/// Spawns EROFS conversion tasks up to the concurrency limit.
fn spawn_batch(
    join_set: &mut tokio::task::JoinSet<Result<(String, Vec<u8>)>>,
    iter: &mut impl Iterator<Item = PathBuf>,
) {
    while join_set.len() < MAX_CONCURRENT {
        let Some(ext) = iter.next() else {
            return;
        };
        join_set.spawn(process_one(ext));
    }
}

/// Converts a single extension directory into a named EROFS blob.
async fn process_one(ext: PathBuf) -> Result<(String, Vec<u8>)> {
    let name = ext
        .file_name()
        .ok_or_else(|| {
            RamuneError::ErofsError(format!("path has no file name: {}", ext.display()))
        })?
        .to_string_lossy()
        .into_owned();

    let erofs_data = tokio::task::spawn_blocking(move || erofs::create(&ext, None))
        .await
        .map_err(|e| RamuneError::ErofsError(e.to_string()))??;

    Ok((format!("extensions/{name}.erofs"), erofs_data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn process_all_empty() {
        // ARRANGE & ACT
        let result = process_all(&[]).await.expect("process_all");

        // ASSERT
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn process_all_single_extension() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("file.txt"), b"data").expect("write");
        let ext_path = dir.path().to_path_buf();
        let name = ext_path.file_name().unwrap().to_string_lossy().into_owned();

        // ACT
        let files = process_all(&[ext_path]).await.expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, format!("extensions/{name}.erofs"));
        assert!(!files[0].1.is_empty());
    }

    #[tokio::test]
    async fn process_all_multiple_extensions() {
        // ARRANGE
        let dirs: Vec<_> = (0..4)
            .map(|i| {
                let d = tempfile::tempdir().expect("tempdir");
                std::fs::write(d.path().join("f.txt"), format!("data{i}")).expect("write");
                d
            })
            .collect();
        let paths: Vec<PathBuf> = dirs.iter().map(|d| d.path().to_path_buf()).collect();

        // ACT
        let files = process_all(&paths).await.expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 4);
        for (path, data) in &files {
            assert!(path.starts_with("extensions/"));
            assert!(path.ends_with(".erofs"));
            assert!(!data.is_empty());
        }
    }

    #[tokio::test]
    async fn process_all_exceeds_concurrency_limit() {
        // ARRANGE
        let dirs: Vec<_> = (0..MAX_CONCURRENT + 2)
            .map(|i| {
                let d = tempfile::tempdir().expect("tempdir");
                std::fs::write(d.path().join("x.txt"), format!("{i}")).expect("write");
                d
            })
            .collect();
        let paths: Vec<PathBuf> = dirs.iter().map(|d| d.path().to_path_buf()).collect();

        // ACT
        let files = process_all(&paths).await.expect("process_all");

        // ASSERT
        assert_eq!(files.len(), MAX_CONCURRENT + 2);
    }

    #[tokio::test]
    async fn process_all_path_without_file_name_errors() {
        // ARRANGE
        let paths = vec![PathBuf::from("/")];

        // ACT
        let result = process_all(&paths).await;

        // ASSERT
        assert!(matches!(result, Err(RamuneError::ErofsError(_))));
    }
}
