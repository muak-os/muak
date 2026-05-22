//! Initramfs extension by appending a compressed archive of EROFS images.

use std::io::Write;
use std::path::Path;

use crate::error::{RamuneError, Result};

/// Settings for appending extension archives to an initramfs image.
pub struct ExtendConfig<'a> {
    /// Path to the base initramfs image to extend.
    pub base: &'a Path,
    /// List of (extension name, extension directory) pairs to include in the appended archive.
    pub extensions: &'a [(String, std::path::PathBuf)],
    /// Zstd compression level for the outer appended extensions archive.
    pub compression_level: i32,
    /// Zstd compression level for each generated extension EROFS image.
    pub extension_compression_level: i32,
}

/// Builds an initramfs by appending a zstd-compressed EROFS extension archive to a base image.
pub async fn extend(config: &ExtendConfig<'_>, output: &Path) -> Result<()> {
    let same_file = is_same_file(config.base, output).await;

    if !same_file {
        tokio::fs::copy(config.base, output)
            .await
            .map_err(|e| RamuneError::ReadError {
                file: config.base.display().to_string(),
                source: e,
            })?;
    }

    if config.extensions.is_empty() {
        return Ok(());
    }

    let archive = build_extensions_archive(config).await?;
    append_to_file(output, &archive).await
}

async fn is_same_file(a: &Path, b: &Path) -> bool {
    match (
        tokio::fs::canonicalize(a).await,
        tokio::fs::canonicalize(b).await,
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

async fn build_extensions_archive(config: &ExtendConfig<'_>) -> Result<Vec<u8>> {
    let files =
        crate::extension::process_all(config.extensions, config.extension_compression_level)
            .await?;
    write_compressed_extensions_archive(Vec::new(), &files, config.compression_level)
}

fn write_compressed_extensions_archive<W: Write>(
    writer: W,
    files: &[(String, Vec<u8>)],
    compression_level: i32,
) -> Result<W> {
    write_compressed_extensions_archive_with(
        writer,
        files,
        compression_level,
        crate::cpio::write_archive::<zstd::Encoder<'static, W>>,
    )
}

fn write_compressed_extensions_archive_with<W, WriteArchive>(
    writer: W,
    files: &[(String, Vec<u8>)],
    compression_level: i32,
    write_archive: WriteArchive,
) -> Result<W>
where
    W: Write,
    WriteArchive: FnOnce(&mut zstd::Encoder<'static, W>, &[(String, Vec<u8>)]) -> Result<()>,
{
    let compression_level = crate::validate_compression_level(compression_level)?;
    let mut encoder =
        zstd::Encoder::new(writer, compression_level).map_err(RamuneError::ZstdInitError)?;

    match write_archive(&mut encoder, files) {
        Ok(()) => encoder.finish().map_err(RamuneError::CompressionError),
        Err(error) => Err(error),
    }
}

async fn append_to_file(path: &Path, data: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
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
    W: tokio::io::AsyncWrite + Unpin,
{
    tokio::io::AsyncWriteExt::write_all(writer, data)
        .await
        .map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })?;
    tokio::io::AsyncWriteExt::flush(writer)
        .await
        .map_err(|source| RamuneError::WriteError {
            file: path.display().to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::*;

    struct CountingFailingWriter {
        fail_on_call: usize,
        calls: usize,
    }

    struct AsyncWriteWriteFailingWriter;

    struct AsyncWriteFlushFailingWriter;

    impl Write for CountingFailingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            let should_fail = self.calls == self.fail_on_call;

            match should_fail {
                true => Err(std::io::Error::other("write failed")),
                false => Ok(buf.len()),
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tokio::io::AsyncWrite for AsyncWriteWriteFailingWriter {
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

    impl tokio::io::AsyncWrite for AsyncWriteFlushFailingWriter {
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

    fn extension_files() -> Vec<(String, Vec<u8>)> {
        vec![("extensions/test-ext.erofs".to_string(), b"payload".to_vec())]
    }

    fn fail_cpio_archive<W>(
        _: &mut zstd::Encoder<'static, W>,
        _: &[(String, Vec<u8>)],
    ) -> Result<()>
    where
        W: Write,
    {
        Err(RamuneError::CpioError("cpio failed".to_string()))
    }

    #[test]
    fn counting_failing_writer_success_paths() {
        use std::io::Write as _;

        let mut writer = CountingFailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        assert_eq!(writer.write(b"x").expect("write"), 1);
        writer.flush().expect("flush");
    }

    #[tokio::test]
    async fn async_write_write_failing_writer_supports_flush_and_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        let mut writer = AsyncWriteWriteFailingWriter;

        writer.flush().await.expect("flush");
        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn async_write_flush_failing_writer_supports_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        let mut writer = AsyncWriteFlushFailingWriter;

        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn extend_initramfs_no_extensions() {
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let config = ExtendConfig {
            base: base.path(),
            extensions: &[],
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        extend(&config, output.path())
            .await
            .expect("extend_initramfs");

        let content = std::fs::read(output.path()).expect("read output");
        assert_eq!(content, b"base-initramfs-content");
    }

    #[tokio::test]
    async fn extend_initramfs_with_extensions() {
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extensions = [("test-ext".to_string(), ext_dir.path().to_path_buf())];
        let config = ExtendConfig {
            base: base.path(),
            extensions: &extensions,
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        extend(&config, output.path())
            .await
            .expect("extend_initramfs");

        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_accepts_separate_extension_compression_level() {
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extensions = [("test-ext".to_string(), ext_dir.path().to_path_buf())];
        let config = ExtendConfig {
            base: base.path(),
            extensions: &extensions,
            compression_level: 19,
            extension_compression_level: 7,
        };

        extend(&config, output.path()).await.expect("extend");

        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_initramfs_same_file_no_extensions() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"initramfs-content").expect("write");

        let config = ExtendConfig {
            base: file.path(),
            extensions: &[],
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        extend(&config, file.path())
            .await
            .expect("extend_initramfs");

        let content = std::fs::read(file.path()).expect("read");
        assert_eq!(content, b"initramfs-content");
    }

    #[tokio::test]
    async fn extend_initramfs_same_file_with_extensions() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"base").expect("write");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extensions = [("test-ext".to_string(), ext_dir.path().to_path_buf())];
        let config = ExtendConfig {
            base: file.path(),
            extensions: &extensions,
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        extend(&config, file.path())
            .await
            .expect("extend_initramfs");

        let content = std::fs::read(file.path()).expect("read");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_initramfs_missing_base() {
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let config = ExtendConfig {
            base: Path::new("/nonexistent/base.img"),
            extensions: &[],
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        let result = extend(&config, output.path()).await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[tokio::test]
    async fn extend_initramfs_missing_extension_errors() {
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let extensions = [(
            "missing".to_string(),
            std::path::PathBuf::from("/nonexistent/extension-dir"),
        )];
        let config = ExtendConfig {
            base: base.path(),
            extensions: &extensions,
            compression_level: 19,
            extension_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        };

        let result = extend(&config, output.path()).await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ErofsError(_)))
        );
    }

    #[tokio::test]
    async fn extend_invalid_extension_compression_level_errors() {
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        let extensions = [("test-ext".to_string(), ext_dir.path().to_path_buf())];
        let config = ExtendConfig {
            base: base.path(),
            extensions: &extensions,
            compression_level: 19,
            extension_compression_level: i32::MAX,
        };

        let result = extend(&config, output.path()).await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_file_writes_data() {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(f.path(), b"hello").expect("write");

        append_to_file(f.path(), b" world").await.expect("append");

        assert_eq!(std::fs::read(f.path()).expect("read"), b"hello world");
    }

    #[tokio::test]
    async fn append_to_file_nonexistent_path_errors() {
        let result = append_to_file(Path::new("/nonexistent/file"), b"data").await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_writer_errors_on_write() {
        let mut writer = AsyncWriteWriteFailingWriter;

        let result = append_to_writer(Path::new("/virtual/output"), &mut writer, b"data").await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_writer_errors_on_flush() {
        let mut writer = AsyncWriteFlushFailingWriter;

        let result = append_to_writer(Path::new("/virtual/output"), &mut writer, b"data").await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[tokio::test]
    async fn append_to_file_read_only_file_errors_on_open() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let metadata = std::fs::metadata(file.path()).expect("metadata");
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(file.path(), permissions).expect("set readonly");

        let result = append_to_file(file.path(), b"data").await;

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::WriteError { .. }))
        );
    }

    #[test]
    fn write_compressed_extensions_archive_invalid_level_errors() {
        let result = write_compressed_extensions_archive(Vec::new(), &extension_files(), i32::MAX);

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn write_compressed_extensions_archive_cpio_errors() {
        let result = write_compressed_extensions_archive_with(
            Vec::new(),
            &extension_files(),
            19,
            fail_cpio_archive::<Vec<u8>>,
        );

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(_)))
        );
    }

    #[test]
    fn write_compressed_extensions_archive_finish_errors() {
        let files = extension_files();
        let mut encoder = zstd::Encoder::new(
            CountingFailingWriter {
                fail_on_call: usize::MAX,
                calls: 0,
            },
            19,
        )
        .expect("encoder");
        crate::cpio::write_archive(&mut encoder, &files).expect("write archive");
        let fail_on_call = encoder.get_ref().calls + 1;

        let result = write_compressed_extensions_archive(
            CountingFailingWriter {
                fail_on_call,
                calls: 0,
            },
            &files,
            19,
        );

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CompressionError(_)))
        );
    }
}
