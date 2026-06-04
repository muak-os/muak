//! Concurrent processing of `ExtraFile` entries into EROFS blobs or raw byte copies.

use std::path::Path;

use tokio::task::JoinSet;

use crate::erofs;
use crate::error::{RamuneError, Result};
use crate::extender::ExtraFile;

pub(crate) const MAX_CONCURRENT: usize = 8;

type ProcessOutput = (String, Vec<u8>);

/// Processes a list of `ExtraFile` entries concurrently and deterministically.
pub(crate) async fn process_extra_files(
    extra_files: &[&ExtraFile<'_>],
    compression_level: i32,
) -> Result<Vec<ProcessOutput>> {
    let mut files = Vec::with_capacity(extra_files.len());

    for batch in extra_files.chunks(MAX_CONCURRENT) {
        files.extend(process_batch(batch, compression_level).await?);
    }

    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

async fn process_batch(
    batch: &[&ExtraFile<'_>],
    compression_level: i32,
) -> Result<Vec<ProcessOutput>> {
    let mut tasks = JoinSet::new();

    for entry in batch {
        let name = entry.name.clone();
        let path = entry.path.to_path_buf();
        let compress = entry.compress;

        tasks.spawn_blocking(move || process_one(&name, &path, compress, compression_level));
    }

    let mut files = Vec::with_capacity(batch.len());

    while let Some(result) = tasks.join_next().await {
        files.push(result.map_err(RamuneError::TaskError)??);
    }

    Ok(files)
}

fn process_one(
    name: &str,
    path: &Path,
    compress: bool,
    compression_level: i32,
) -> Result<ProcessOutput> {
    if compress {
        erofs::create(path, None, compression_level).map(|data| (name.to_owned(), data))
    } else {
        let data = std::fs::read(path).map_err(|source| RamuneError::ReadError {
            file: path.display().to_string(),
            source,
        })?;
        Ok((name.to_owned(), data))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::error::RamuneError;

    fn make_extension_dir(name: &str, data: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(name), data).expect("write");
        dir
    }

    fn make_temp_file(data: &[u8]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), data).expect("write");
        file
    }

    fn extra_file<'a>(name: &str, path: &'a Path, compress: bool) -> ExtraFile<'a> {
        ExtraFile {
            name: name.to_owned(),
            path,
            compress,
        }
    }

    #[tokio::test]
    async fn process_extra_files_empty() {
        // ARRANGE
        let entries: &[&ExtraFile<'_>] = &[];

        // ACT
        let result = process_extra_files(entries, 3).await.expect("process");

        // ASSERT
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn process_extra_files_compress_dir() {
        // ARRANGE
        let dir = make_extension_dir("file.txt", b"data");
        let entry = extra_file("extensions/test.erofs", dir.path(), true);
        let entries = [&entry];

        // ACT
        let files = process_extra_files(&entries, 3).await.expect("process");

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(files.first().expect("first").0, "extensions/test.erofs");
        assert!(!files.first().expect("first").1.is_empty());
    }

    #[tokio::test]
    async fn process_extra_files_plain_file() {
        // ARRANGE
        let file = make_temp_file(b"profile data");
        let entry = extra_file("profile.toml", file.path(), false);
        let entries = [&entry];

        // ACT
        let files = process_extra_files(&entries, 3).await.expect("process");

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(files.first().expect("first").0, "profile.toml");
        assert_eq!(files.first().expect("first").1, b"profile data");
    }

    #[tokio::test]
    async fn process_extra_files_mixed_sorted() {
        // ARRANGE
        let dir = make_extension_dir("f.txt", b"ext");
        let file = make_temp_file(b"raw");

        let entries = [
            &extra_file("extensions/z.erofs", dir.path(), true),
            &extra_file("a.txt", file.path(), false),
        ];

        // ACT
        let files = process_extra_files(&entries, 3).await.expect("process");

        // ASSERT
        assert_eq!(files.len(), 2);
        assert_eq!(files.first().expect("first").0, "a.txt");
        assert_eq!(files.get(1).expect("second").0, "extensions/z.erofs");
    }

    #[tokio::test]
    async fn process_extra_files_missing_source_errors() {
        // ARRANGE
        let missing = PathBuf::from("/nonexistent/extra-source");
        let entry = extra_file("missing.erofs", &missing, true);
        let entries = [&entry];

        // ACT
        let result = process_extra_files(&entries, 3).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ErofsError(_)))
        );
    }

    #[tokio::test]
    async fn process_extra_files_invalid_compression_errors() {
        // ARRANGE
        let dir = make_extension_dir("f.txt", b"data");
        let entry = extra_file("ext.erofs", dir.path(), true);
        let entries = [&entry];

        // ACT
        let result = process_extra_files(&entries, i32::MAX).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[tokio::test]
    #[expect(
        clippy::excessive_nesting,
        reason = "spawn_blocking requires an extra closure nesting level"
    )]
    async fn process_extra_files_task_panic_errors() {
        // ARRANGE
        let dir = make_extension_dir("file.txt", b"data");
        let entry = extra_file("panic-ext.erofs", dir.path(), true);

        let mut tasks = JoinSet::new();
        let name = entry.name.clone();
        let path = entry.path.to_path_buf();
        let compress = entry.compress;
        tasks.spawn_blocking(move || -> Result<ProcessOutput> {
            drop((name, path, compress));
            panic!("task panicked")
        });

        // ACT
        let result: Result<Vec<ProcessOutput>> = async {
            let mut files = Vec::new();
            while let Some(result) = tasks.join_next().await {
                files.push(result.map_err(RamuneError::TaskError)??);
            }
            Ok(files)
        }
        .await;

        // ASSERT
        assert!(matches!(
            result.as_ref(),
            Err(RamuneError::TaskError(error)) if error.is_panic()
        ));
    }

    #[tokio::test]
    async fn process_extra_files_exceeds_concurrency_limit() {
        // ARRANGE
        let dirs: Vec<_> = (0..MAX_CONCURRENT.saturating_add(2))
            .map(|index| make_extension_dir("x.txt", format!("{index}").as_bytes()))
            .collect();
        let entries: Vec<ExtraFile<'_>> = dirs
            .iter()
            .enumerate()
            .map(|(index, dir)| extra_file(&format!("ext-{index:02}.erofs"), dir.path(), true))
            .collect();
        let refs: Vec<&ExtraFile<'_>> = entries.iter().collect();

        // ACT
        let files = process_extra_files(&refs, 3).await.expect("process");

        // ASSERT
        assert_eq!(files.len(), MAX_CONCURRENT.saturating_add(2));
    }
}
