//! Concurrent processing of pre-extracted extension directories into named EROFS blobs.

use std::path::{Path, PathBuf};

use tokio::task::JoinSet;

use crate::erofs;
use crate::error::{RamuneError, Result};

/// Maximum number of extensions processed concurrently.
pub(crate) const MAX_CONCURRENT: usize = 8;

type ProcessFn = fn(&str, &Path, i32) -> Result<(String, Vec<u8>)>;

type ProcessOutput = (String, Vec<u8>);

/// Processes all extension directories concurrently and deterministically.
pub(crate) async fn process_all(
    extensions: &[(String, PathBuf)],
    compression_level: i32,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::with_capacity(extensions.len());

    for batch in extensions.chunks(MAX_CONCURRENT) {
        files.extend(process_batch(batch, compression_level, process).await?);
    }

    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

async fn process_batch(
    batch: &[(String, PathBuf)],
    compression_level: i32,
    process: ProcessFn,
) -> Result<Vec<ProcessOutput>> {
    let mut tasks = JoinSet::new();

    for entry in batch {
        let name = entry.0.clone();
        let path = entry.1.clone();

        tasks.spawn_blocking(move || process(&name, &path, compression_level));
    }

    let mut files = Vec::with_capacity(batch.len());

    while let Some(result) = tasks.join_next().await {
        files.push(result.map_err(RamuneError::ExtensionTaskError)??);
    }

    Ok(files)
}

/// Converts a single extension directory into a named EROFS blob.
fn process(name: &str, path: &Path, compression_level: i32) -> Result<(String, Vec<u8>)> {
    erofs::create(path, None, compression_level)
        .map(|erofs_data| (format!("extensions/{name}.erofs"), erofs_data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RamuneError;

    fn make_extension_dir(name: &str, data: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(name), data).expect("write");
        dir
    }

    #[tokio::test]
    async fn process_all_empty() {
        // ARRANGE & ACT
        let result = process_all(&[], ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .await
            .expect("process_all");

        // ASSERT
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn process_all_single_extension() {
        // ARRANGE
        let dir = make_extension_dir("file.txt", b"data");

        // ACT
        let files = process_all(
            &[("muak-os-iscsi-tools".to_owned(), dir.path().to_path_buf())],
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        )
        .await
        .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 1);
        let file = files.first().expect("expected one extension file");
        assert_eq!(file.0, "extensions/muak-os-iscsi-tools.erofs");
        assert!(!file.1.is_empty());
    }

    #[tokio::test]
    async fn process_all_multiple_extensions_sorted() {
        // ARRANGE
        let names = ["zebra", "alpha", "mango", "beta"];
        let dirs: Vec<_> = names
            .iter()
            .map(|name| make_extension_dir("f.txt", name.as_bytes()))
            .collect();
        let inputs: Vec<(String, PathBuf)> = names
            .iter()
            .zip(dirs.iter())
            .map(|(name, dir)| ((*name).to_owned(), dir.path().to_path_buf()))
            .collect();

        // ACT
        let files = process_all(&inputs, ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .await
            .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 4);
        let archive_names: Vec<&str> = files.iter().map(|file| file.0.as_str()).collect();
        assert_eq!(
            archive_names,
            [
                "extensions/alpha.erofs",
                "extensions/beta.erofs",
                "extensions/mango.erofs",
                "extensions/zebra.erofs",
            ]
        );
    }

    #[tokio::test]
    async fn process_all_exceeds_concurrency_limit() {
        // ARRANGE
        let dirs: Vec<_> = (0..MAX_CONCURRENT.saturating_add(2))
            .map(|index| make_extension_dir("x.txt", format!("{index}").as_bytes()))
            .collect();
        let entries: Vec<(String, PathBuf)> = dirs
            .iter()
            .enumerate()
            .map(|(index, dir)| (format!("ext-{index:02}"), dir.path().to_path_buf()))
            .collect();

        // ACT
        let files = process_all(&entries, ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .await
            .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), MAX_CONCURRENT.saturating_add(2));
    }

    #[tokio::test]
    async fn process_all_missing_extension_errors() {
        // ARRANGE
        let missing = PathBuf::from("/nonexistent/extension-dir");

        // ACT
        let result = process_all(
            &[("missing".to_owned(), missing)],
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        )
        .await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ErofsError(_)))
        );
    }

    #[tokio::test]
    async fn process_all_task_panic_errors() {
        // ARRANGE
        let dir = make_extension_dir("file.txt", b"data");
        let extensions = [("panic-ext".to_owned(), dir.path().to_path_buf())];

        // ACT
        let result = process_batch(
            &extensions,
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
            |_, _, _| panic!("extension task panicked"),
        )
        .await;

        // ASSERT
        assert!(matches!(
            result.as_ref(),
            Err(RamuneError::ExtensionTaskError(error)) if error.is_panic()
        ));
    }
}
