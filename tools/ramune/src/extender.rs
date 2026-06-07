//! Initramfs extension by appending a compressed archive of extra files.

use std::io::Write;
use std::path::Path;

use tokio::fs::{OpenOptions, canonicalize, copy};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::compress;
use crate::cpio;
use crate::error::{RamuneError, Result};
use crate::extra::process_extra_files;

/// Settings for appending a compressed extrafile archive to an initramfs image.
pub struct ExtendConfig<'a> {
    /// Path to the base initramfs image to extend.
    pub base: &'a Path,
    /// Entries to include in the appended archive.
    pub extra_files: &'a [ExtraFile<'a>],
    /// Zstd compression level for the appended archive and any EROFS conversion.
    pub compression_level: i32,
}

/// A single entry to append to the initramfs.
pub struct ExtraFile<'a> {
    /// Destination path inside the appended CPIO archive.
    pub name: String,
    /// Source path on disk.
    pub path: &'a Path,
    /// When true, convert the source to a zstd-compressed EROFS blob before packing.
    pub compress: bool,
}

/// Builds an initramfs by appending a zstd-compressed archive of extra files to a base image.
///
/// # Errors
///
/// Returns an error when validation of extra files fails, the base image cannot be read,
/// extra files cannot be processed, compression fails, or the output cannot be written.
pub async fn extend(config: &ExtendConfig<'_>, output: &Path) -> Result<()> {
    let mut sorted: Vec<&ExtraFile<'_>> = config.extra_files.iter().collect();
    sorted.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    let sorted = sorted.as_slice();

    validate_extra_files(sorted)?;

    let same_file = is_same_file(config.base, output).await;

    if !same_file {
        copy(config.base, output)
            .await
            .map_err(|e| RamuneError::ReadError {
                file: config.base.display().to_string(),
                source: e,
            })?;
    }

    if sorted.is_empty() {
        return Ok(());
    }

    let archive = build_extra_archive(sorted, config.compression_level).await?;
    append_to_file(output, &archive).await
}

fn validate_extra_files(extra_files: &[&ExtraFile<'_>]) -> Result<()> {
    let mut prev: Option<&str> = None;

    for entry in extra_files {
        if entry.name.is_empty() {
            return Err(RamuneError::CpioError(
                "extra file name must not be empty".to_owned(),
            ));
        }

        if entry.name.starts_with('/') {
            return Err(RamuneError::CpioError(format!(
                "extra file name must not be absolute: {}",
                entry.name
            )));
        }

        if entry.name.contains("..") {
            return Err(RamuneError::CpioError(format!(
                "extra file name must not contain ..: {}",
                entry.name
            )));
        }

        if let Some(previous_name) = prev
            && entry.name.as_str() == previous_name
        {
            return Err(RamuneError::CpioError(format!(
                "duplicate extra file name: {}",
                entry.name
            )));
        }

        prev = Some(&entry.name);
    }

    Ok(())
}

async fn is_same_file(first: &Path, second: &Path) -> bool {
    match (canonicalize(first).await, canonicalize(second).await) {
        (Ok(first_path), Ok(second_path)) => first_path == second_path,
        _ => false,
    }
}

async fn build_extra_archive(
    extra_files: &[&ExtraFile<'_>],
    compression_level: i32,
) -> Result<Vec<u8>> {
    let files = process_extra_files(extra_files, compression_level).await?;
    write_compressed_cpio_archive(Vec::new(), &files, compression_level)
}

fn write_compressed_cpio_archive<W: Write>(
    writer: W,
    files: &[(String, Vec<u8>)],
    compression_level: i32,
) -> Result<W> {
    let mut encoder = compress::encoder(writer, compression_level)?;
    let mut ino = 1_u32;

    for abs_rel in files {
        let (path, data) = (abs_rel.0.as_str(), abs_rel.1.as_slice());
        let size = u32::try_from(data.len())
            .map_err(|_err| RamuneError::CpioError("extra file exceeds CPIO limits".to_owned()))?;
        cpio::write_entry(&mut encoder, ino, path, 0o100_644, size, |w| {
            w.write_all(data)
                .map_err(|e| RamuneError::CpioError(format!("{e}")))
        })?;
        ino = ino
            .checked_add(1)
            .ok_or_else(|| RamuneError::CpioError("CPIO inode overflowed".to_owned()))?;
    }

    cpio::write_trailer(&mut encoder)?;

    encoder.finish().map_err(RamuneError::CompressionError)
}

async fn append_to_file(path: &Path, data: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })?;

    append_to_writer(path, &mut file, data).await
}

async fn append_to_writer<W>(path: &Path, writer: &mut W, data: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer
        .write_all(data)
        .await
        .map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })?;
    writer
        .flush()
        .await
        .map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use core::pin::Pin;
    use core::task::{Context, Poll};

    use tokio::io::AsyncWrite;

    use super::*;

    struct CountingFailingWriter {
        fail_on_call: usize,
        calls: usize,
    }

    struct AsyncWriteWriteFailingWriter;

    struct AsyncWriteFlushFailingWriter;

    impl Write for CountingFailingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls = self.calls.saturating_add(1);

            (self.calls != self.fail_on_call)
                .then_some(buf.len())
                .ok_or_else(|| std::io::Error::other("write failed"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl AsyncWrite for AsyncWriteWriteFailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::Error::other("write failed")))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for AsyncWriteFlushFailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::other("flush failed")))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn extra<'a>(name: &str, path: &'a Path, compress: bool) -> ExtraFile<'a> {
        ExtraFile {
            name: name.to_owned(),
            path,
            compress,
        }
    }

    #[test]
    fn counting_failing_writer_success_paths() {
        use std::io::Write as _;

        // ARRANGE
        let mut writer = CountingFailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        // ACT
        assert_eq!(writer.write(b"x").expect("write"), 1);
        // ASSERT
        writer.flush().expect("flush");
    }

    #[tokio::test]
    async fn async_write_write_failing_writer_supports_flush_and_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        // ARRANGE
        let mut writer = AsyncWriteWriteFailingWriter;

        // ACT
        writer.flush().await.expect("flush");
        // ASSERT
        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn async_write_flush_failing_writer_supports_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        // ARRANGE
        let mut writer = AsyncWriteFlushFailingWriter;

        // ACT
        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn extend_no_extra_files() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let config = ExtendConfig {
            base: base.path(),
            extra_files: &[],
            compression_level: 19,
        };

        // ACT
        extend(&config, output.path()).await.expect("extend");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert_eq!(content, b"base-initramfs-content");
    }

    #[tokio::test]
    async fn extend_with_compress_dir() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extras = [extra("extensions/test-ext.erofs", ext_dir.path(), true)];
        let config = ExtendConfig {
            base: base.path(),
            extra_files: &extras,
            compression_level: 19,
        };

        // ACT
        extend(&config, output.path()).await.expect("extend");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_with_plain_file() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let file = tempfile::NamedTempFile::new().expect("extra file");
        std::fs::write(file.path(), b"profile content").expect("write extra");

        let extras = [extra("profile.toml", file.path(), false)];
        let config = ExtendConfig {
            base: base.path(),
            extra_files: &extras,
            compression_level: 19,
        };

        // ACT
        extend(&config, output.path()).await.expect("extend");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_same_file_no_extra_files() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"initramfs-content").expect("write");

        let config = ExtendConfig {
            base: file.path(),
            extra_files: &[],
            compression_level: 19,
        };

        // ACT
        extend(&config, file.path()).await.expect("extend");

        // ASSERT
        let content = std::fs::read(file.path()).expect("read");
        assert_eq!(content, b"initramfs-content");
    }

    #[tokio::test]
    async fn extend_same_file_with_compress_dir() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"base").expect("write");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extras = [extra("extensions/test-ext.erofs", ext_dir.path(), true)];
        let config = ExtendConfig {
            base: file.path(),
            extra_files: &extras,
            compression_level: 19,
        };

        // ACT
        extend(&config, file.path()).await.expect("extend");

        // ASSERT
        let content = std::fs::read(file.path()).expect("read");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_missing_base() {
        // ARRANGE
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let config = ExtendConfig {
            base: Path::new("/nonexistent/base.img"),
            extra_files: &[],
            compression_level: 19,
        };

        // ACT
        let result = extend(&config, output.path()).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[tokio::test]
    async fn extend_missing_compress_source_errors() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let extras = [extra(
            "missing.erofs",
            Path::new("/nonexistent/extra-source"),
            true,
        )];
        let config = ExtendConfig {
            base: base.path(),
            extra_files: &extras,
            compression_level: 19,
        };

        // ACT
        let result = extend(&config, output.path()).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ErofsError(_)))
        );
    }

    #[tokio::test]
    async fn extend_missing_plain_source_errors() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let extras = [extra(
            "profile.toml",
            Path::new("/nonexistent/extra-source"),
            false,
        )];
        let config = ExtendConfig {
            base: base.path(),
            extra_files: &extras,
            compression_level: 19,
        };

        // ACT
        let result = extend(&config, output.path()).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[tokio::test]
    async fn extend_invalid_compression_level_errors() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extras = [extra("extensions/test-ext.erofs", ext_dir.path(), true)];
        let config = ExtendConfig {
            base: base.path(),
            extra_files: &extras,
            compression_level: i32::MAX,
        };

        // ACT
        let result = extend(&config, output.path()).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn validate_rejects_empty_name() {
        // ARRANGE
        let entry = ExtraFile {
            name: String::new(),
            path: Path::new("/tmp/x"),
            compress: false,
        };
        let extras = [&entry];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("must not be empty"))
        );
    }

    #[test]
    fn validate_rejects_absolute_name() {
        // ARRANGE
        let entry = ExtraFile {
            name: "/absolute/path".to_owned(),
            path: Path::new("/tmp/x"),
            compress: false,
        };
        let extras = [&entry];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("must not be absolute"))
        );
    }

    #[test]
    fn validate_rejects_dotdot() {
        // ARRANGE
        let entry = ExtraFile {
            name: "foo/../bar".to_owned(),
            path: Path::new("/tmp/x"),
            compress: false,
        };
        let extras = [&entry];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("must not contain .."))
        );
    }

    #[test]
    fn validate_rejects_duplicates() {
        // ARRANGE
        let e1 = ExtraFile {
            name: "a.txt".to_owned(),
            path: Path::new("/tmp/a"),
            compress: false,
        };
        let e2 = ExtraFile {
            name: "a.txt".to_owned(),
            path: Path::new("/tmp/b"),
            compress: false,
        };
        let extras = [&e1, &e2];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("duplicate"))
        );
    }

    #[test]
    fn validate_accepts_valid_entries() {
        // ARRANGE
        let e1 = ExtraFile {
            name: "a.txt".to_owned(),
            path: Path::new("/tmp/a"),
            compress: false,
        };
        let e2 = ExtraFile {
            name: "b.txt".to_owned(),
            path: Path::new("/tmp/b"),
            compress: true,
        };
        let extras = [&e1, &e2];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn validate_accepts_extensions_path() {
        // ARRANGE
        let e1 = ExtraFile {
            name: "extensions/test.erofs".to_owned(),
            path: Path::new("/tmp/ext"),
            compress: true,
        };
        let e2 = ExtraFile {
            name: "profile.toml".to_owned(),
            path: Path::new("/tmp/profile"),
            compress: false,
        };
        let extras = [&e1, &e2];

        // ACT
        let result = validate_extra_files(&extras);

        // ASSERT
        result.unwrap();
    }

    #[tokio::test]
    async fn append_to_file_writes_data() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"hello").expect("write");

        // ACT
        append_to_file(file.path(), b" world")
            .await
            .expect("append");

        // ASSERT
        assert_eq!(std::fs::read(file.path()).expect("read"), b"hello world");
    }

    #[tokio::test]
    async fn append_to_file_nonexistent_path_errors() {
        // ARRANGE
        let path = Path::new("/nonexistent/file");

        // ACT
        let result = append_to_file(path, b"data").await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_writer_errors_on_write() {
        // ARRANGE
        let mut writer = AsyncWriteWriteFailingWriter;

        // ACT
        let result = append_to_writer(Path::new("/virtual/output"), &mut writer, b"data").await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_writer_errors_on_flush() {
        // ARRANGE
        let mut writer = AsyncWriteFlushFailingWriter;

        // ACT
        let result = append_to_writer(Path::new("/virtual/output"), &mut writer, b"data").await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_file_read_only_file_errors_on_open() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let metadata = std::fs::metadata(file.path()).expect("metadata");
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(file.path(), permissions).expect("set readonly");

        // ACT
        let result = append_to_file(file.path(), b"data").await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn write_compressed_cpio_invalid_level_errors() {
        // ARRANGE
        let files = vec![("test.erofs".to_owned(), b"data".to_vec())];

        // ACT
        let result = write_compressed_cpio_archive(Vec::new(), &files, i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn write_compressed_cpio_finish_errors() {
        // ARRANGE
        let files = vec![("test.erofs".to_owned(), b"data".to_vec())];
        let mut clean = Vec::new();

        // ACT
        write_compressed_cpio_archive(&mut clean, &files, 19).expect("write");
        let result = write_compressed_cpio_archive(
            CountingFailingWriter {
                fail_on_call: 1,
                calls: 0,
            },
            &files,
            19,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CompressionError(_)))
        );
    }
}
