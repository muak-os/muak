//! Initramfs extension by appending a compressed archive of EROFS images.

use std::io::Write;
use std::path::Path;

use tokio::fs::{OpenOptions, canonicalize, copy};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::compress;
use crate::cpio::{CpioEntry, write_archive as write_cpio_archive};
use crate::error::{RamuneError, Result};
use crate::extension::process_all;

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
///
/// # Errors
///
/// Returns an error when reading the base image, processing extensions, compressing the
/// appended archive, or writing the output image fails.
pub async fn extend(config: &ExtendConfig<'_>, output: &Path) -> Result<()> {
    let same_file = is_same_file(config.base, output).await;

    if !same_file {
        copy(config.base, output)
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

async fn is_same_file(first: &Path, second: &Path) -> bool {
    match (canonicalize(first).await, canonicalize(second).await) {
        (Ok(first_path), Ok(second_path)) => first_path == second_path,
        _ => false,
    }
}

async fn build_extensions_archive(config: &ExtendConfig<'_>) -> Result<Vec<u8>> {
    let files = process_all(config.extensions, config.extension_compression_level).await?;
    let entries = extension_archive_entries(&files);
    write_compressed_extensions_archive(Vec::new(), &entries, config.compression_level)
}

fn write_compressed_extensions_archive<W: Write>(
    writer: W,
    entries: &[CpioEntry<'_>],
    compression_level: i32,
) -> Result<W> {
    write_compressed_extensions_archive_with(
        writer,
        entries,
        compression_level,
        write_cpio_archive::<zstd::Encoder<'static, W>>,
    )
}

fn write_compressed_extensions_archive_with<W, WriteArchive>(
    writer: W,
    entries: &[CpioEntry<'_>],
    compression_level: i32,
    write_archive: WriteArchive,
) -> Result<W>
where
    W: Write,
    WriteArchive: FnOnce(&mut zstd::Encoder<'static, W>, &[CpioEntry<'_>]) -> Result<()>,
{
    let mut encoder = compress::encoder(writer, compression_level)?;

    match write_archive(&mut encoder, entries) {
        Ok(()) => encoder.finish().map_err(RamuneError::CompressionError),
        Err(error) => Err(error),
    }
}

fn extension_archive_entries(files: &[(String, Vec<u8>)]) -> Vec<CpioEntry<'_>> {
    let mut entries = Vec::with_capacity(files.len().saturating_add(1));
    entries.push(CpioEntry {
        path: "extensions",
        mode: 0o040_755,
        data: &[],
    });
    entries.extend(files.iter().map(|file| CpioEntry {
        path: file.0.as_str(),
        mode: 0o100_644,
        data: file.1.as_slice(),
    }));
    entries
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
    use crate::cpio::write_archive;

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

    fn extension_entries() -> Vec<CpioEntry<'static>> {
        vec![
            CpioEntry {
                path: "extensions",
                mode: 0o040_755,
                data: &[],
            },
            CpioEntry {
                path: "extensions/test-ext.erofs",
                mode: 0o100_644,
                data: b"payload",
            },
        ]
    }

    fn fail_cpio_archive<W>(_: &mut zstd::Encoder<'static, W>, _: &[CpioEntry<'_>]) -> Result<()>
    where
        W: Write,
    {
        Err(RamuneError::CpioError("cpio failed".to_owned()))
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

        let extensions = [("test-ext".to_owned(), ext_dir.path().to_path_buf())];
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

        let extensions = [("test-ext".to_owned(), ext_dir.path().to_path_buf())];
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

        let extensions = [("test-ext".to_owned(), ext_dir.path().to_path_buf())];
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
            "missing".to_owned(),
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

        let extensions = [("test-ext".to_owned(), ext_dir.path().to_path_buf())];
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
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"hello").expect("write");

        append_to_file(file.path(), b" world")
            .await
            .expect("append");

        assert_eq!(std::fs::read(file.path()).expect("read"), b"hello world");
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
        let result =
            write_compressed_extensions_archive(Vec::new(), &extension_entries(), i32::MAX);

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
            &extension_entries(),
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
        let entries = extension_entries();
        let mut encoder = zstd::Encoder::new(
            CountingFailingWriter {
                fail_on_call: usize::MAX,
                calls: 0,
            },
            19,
        )
        .expect("encoder");
        write_archive(&mut encoder, &entries).expect("write archive");
        let fail_on_call = encoder.get_ref().calls.saturating_add(1);

        let result = write_compressed_extensions_archive(
            CountingFailingWriter {
                fail_on_call,
                calls: 0,
            },
            &entries,
            19,
        );

        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CompressionError(_)))
        );
    }
}
