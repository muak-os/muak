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

    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

async fn process_batch(
    batch: &[(String, PathBuf)],
    compression_level: i32,
    process: ProcessFn,
) -> Result<Vec<ProcessOutput>> {
    let mut tasks = JoinSet::new();

    for (name, path) in batch {
        let name = name.clone();
        let path = path.clone();

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

    fn panic_process_one(_: &str, _: &Path, _: i32) -> Result<(String, Vec<u8>)> {
        panic!("extension task panicked");
    }

    fn make_extension_dir(name: &str, data: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(name), data).expect("write");
        dir
    }

    fn make_leaked_extension(i: usize) -> (String, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("x.txt"), format!("{i}")).expect("write");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        (format!("ext-{i:02}"), path)
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
            &[("muak-os-iscsi-tools".to_string(), dir.path().to_path_buf())],
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        )
        .await
        .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "extensions/muak-os-iscsi-tools.erofs");
        assert!(!files[0].1.is_empty());
    }

    #[tokio::test]
    async fn process_all_multiple_extensions_sorted() {
        // ARRANGE
        let names = ["zebra", "alpha", "mango", "beta"];
        let dirs: Vec<_> = names
            .iter()
            .map(|n| make_extension_dir("f.txt", n.as_bytes()))
            .collect();
        let inputs: Vec<(String, PathBuf)> = names
            .iter()
            .zip(dirs.iter())
            .map(|(n, d)| (n.to_string(), d.path().to_path_buf()))
            .collect();

        // ACT
        let files = process_all(&inputs, ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .await
            .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), 4);
        let archive_names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
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
        let entries: Vec<(String, PathBuf)> =
            (0..MAX_CONCURRENT + 2).map(make_leaked_extension).collect();

        // ACT
        let files = process_all(&entries, ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .await
            .expect("process_all");

        // ASSERT
        assert_eq!(files.len(), MAX_CONCURRENT + 2);
    }

    #[tokio::test]
    async fn process_all_missing_extension_errors() {
        // ARRANGE
        let missing = PathBuf::from("/nonexistent/extension-dir");

        // ACT
        let result = process_all(
            &[("missing".to_string(), missing)],
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        )
        .await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, crate::error::RamuneError::ErofsError(_)))
        );
    }

    #[tokio::test]
    async fn process_all_task_panic_errors() {
        // ARRANGE
        let dir = make_extension_dir("file.txt", b"data");
        let extensions = [("panic-ext".to_string(), dir.path().to_path_buf())];

        // ACT
        let result = process_batch(
            &extensions,
            ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
            panic_process_one,
        )
        .await;

        // ASSERT
        assert!(matches!(
            result.as_ref(),
            Err(crate::error::RamuneError::ExtensionTaskError(error)) if error.is_panic()
        ));
    }
}
