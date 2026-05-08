//! Concurrent processing of pre-extracted extension directories into named EROFS blobs.

use std::path::PathBuf;

use crate::erofs;
use crate::error::{RamuneError, Result};

/// Maximum number of extensions processed concurrently.
pub(crate) const MAX_CONCURRENT: usize = 8;

/// Processes all extension directories concurrently and deterministicly.
pub(crate) async fn process_all(
    extensions: &[(String, PathBuf)],
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut join_set = tokio::task::JoinSet::new();
    let mut iter = extensions.iter().cloned();
    let mut files = Vec::with_capacity(extensions.len());

    spawn_batch(&mut join_set, &mut iter);

    while let Some(result) = join_set.join_next().await {
        files.push(result.map_err(|e| RamuneError::ErofsError(e.to_string()))??);
        spawn_batch(&mut join_set, &mut iter);
    }

    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Spawns EROFS conversion tasks up to the concurrency limit.
fn spawn_batch(
    join_set: &mut tokio::task::JoinSet<Result<(String, Vec<u8>)>>,
    iter: &mut impl Iterator<Item = (String, PathBuf)>,
) {
    while join_set.len() < MAX_CONCURRENT {
        let Some((name, path)) = iter.next() else {
            return;
        };
        join_set.spawn(process_one(name, path));
    }
}

/// Converts a single extension directory into a named EROFS blob.
async fn process_one(name: String, path: PathBuf) -> Result<(String, Vec<u8>)> {
    let erofs_data = tokio::task::spawn_blocking(move || erofs::create(&path, None))
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

        // ACT
        let files = process_all(&[("muak-os-iscsi-tools".to_string(), dir.path().to_path_buf())])
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
            .map(|n| {
                let d = tempfile::tempdir().expect("tempdir");
                std::fs::write(d.path().join("f.txt"), n.as_bytes()).expect("write");
                d
            })
            .collect();
        let inputs: Vec<(String, PathBuf)> = names
            .iter()
            .zip(dirs.iter())
            .map(|(n, d)| (n.to_string(), d.path().to_path_buf()))
            .collect();

        // ACT
        let files = process_all(&inputs).await.expect("process_all");

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
        let entries: Vec<(String, PathBuf)> = (0..MAX_CONCURRENT + 2)
            .map(|i| {
                let d = tempfile::tempdir().expect("tempdir");
                std::fs::write(d.path().join("x.txt"), format!("{i}")).expect("write");
                // leak the TempDir so it stays alive for the test duration
                let path = d.path().to_path_buf();
                std::mem::forget(d);
                (format!("ext-{i:02}"), path)
            })
            .collect();

        // ACT
        let files = process_all(&entries).await.expect("process_all");

        // ASSERT
        assert_eq!(files.len(), MAX_CONCURRENT + 2);
    }
}
