//! Ramune: initramfs builder for creating base images and appending extensions.

#[cfg(feature = "cli")]
pub mod cli;
mod cpio;
mod create;
mod erofs;
mod extension;

pub mod error;

use std::io::Write;
use std::path::{Path, PathBuf};

pub use create::CreateConfig;
pub use error::{RamuneError, Result};

/// Creates a base initramfs image from an init binary and rootfs directory.
pub fn create(config: &CreateConfig<'_>, output: &Path) -> Result<()> {
    let data = create::create_initramfs(config)?;
    std::fs::write(output, &data).map_err(|source| RamuneError::WriteError {
        file: output.display().to_string(),
        source,
    })
}

/// Builds an initramfs by appending a zstd-compressed EROFS extension archive to a base image.
pub async fn extend(base: &Path, extensions: &[(String, PathBuf)], output: &Path) -> Result<()> {
    let same_file = is_same_file(base, output).await;

    if !same_file {
        tokio::fs::copy(base, output)
            .await
            .map_err(|e| RamuneError::ReadError {
                file: base.display().to_string(),
                source: e,
            })?;
    }

    if extensions.is_empty() {
        return Ok(());
    }

    let archive = build_extensions_archive(extensions).await?;
    append_to_file(output, &archive).await
}

/// Checks whether two paths refer to the same file on disk.
async fn is_same_file(a: &Path, b: &Path) -> bool {
    match (
        tokio::fs::canonicalize(a).await,
        tokio::fs::canonicalize(b).await,
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Builds a zstd-compressed CPIO archive of EROFS images for each extension directory.
async fn build_extensions_archive(extensions: &[(String, PathBuf)]) -> Result<Vec<u8>> {
    let files = extension::process_all(extensions).await?;
    write_compressed_extensions_archive(Vec::new(), &files, 19)
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
        cpio::write_archive::<zstd::Encoder<'static, W>>,
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
    let compression_level = validate_compression_level(compression_level)?;
    let mut encoder =
        zstd::Encoder::new(writer, compression_level).map_err(RamuneError::ZstdInitError)?;

    match write_archive(&mut encoder, files) {
        Ok(()) => encoder.finish().map_err(RamuneError::CompressionError),
        Err(error) => Err(error),
    }
}

/// Appends data to the file at `path`.
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

pub(crate) fn validate_compression_level(compression_level: i32) -> Result<i32> {
    let range = zstd::compression_level_range();

    if compression_level == 0 || range.contains(&compression_level) {
        Ok(compression_level)
    } else {
        Err(RamuneError::InvalidCompressionLevel {
            level: compression_level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
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

    fn create_config<'a>(init: &'a Path, rootfs_dir: &'a Path) -> CreateConfig<'a> {
        CreateConfig {
            init,
            rootfs_dir,
            file_contexts: None,
            compression_level: 19,
            rootfs_compression_level: ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL,
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

        // ARRANGE
        let mut writer = CountingFailingWriter {
            fail_on_call: usize::MAX,
            calls: 0,
        };

        // ACT / ASSERT
        assert_eq!(writer.write(b"x").expect("write"), 1);
        writer.flush().expect("flush");
    }

    #[tokio::test]
    async fn async_write_write_failing_writer_supports_flush_and_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        // ARRANGE
        let mut writer = AsyncWriteWriteFailingWriter;

        // ACT / ASSERT
        writer.flush().await.expect("flush");
        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn async_write_flush_failing_writer_supports_shutdown() {
        use tokio::io::AsyncWriteExt as _;

        // ARRANGE
        let mut writer = AsyncWriteFlushFailingWriter;

        // ACT / ASSERT
        writer.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn extend_initramfs_no_extensions() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        // ACT
        extend(base.path(), &[], output.path())
            .await
            .expect("extend_initramfs");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert_eq!(content, b"base-initramfs-content");
    }

    #[tokio::test]
    async fn extend_initramfs_with_extensions() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        // ACT
        extend(
            base.path(),
            &[("test-ext".to_string(), ext_dir.path().to_path_buf())],
            output.path(),
        )
        .await
        .expect("extend_initramfs");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_initramfs_same_file_no_extensions() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"initramfs-content").expect("write");

        // ACT
        extend(file.path(), &[], file.path())
            .await
            .expect("extend_initramfs");

        // ASSERT
        let content = std::fs::read(file.path()).expect("read");
        assert_eq!(content, b"initramfs-content");
    }

    #[tokio::test]
    async fn extend_initramfs_same_file_with_extensions() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), b"base").expect("write");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        // ACT
        extend(
            file.path(),
            &[("test-ext".to_string(), ext_dir.path().to_path_buf())],
            file.path(),
        )
        .await
        .expect("extend_initramfs");

        // ASSERT
        let content = std::fs::read(file.path()).expect("read");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn extend_initramfs_missing_base() {
        // ARRANGE
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        // ACT
        let result = extend(Path::new("/nonexistent/base.img"), &[], output.path()).await;

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::ReadError { .. }))
        );
    }

    #[tokio::test]
    async fn extend_initramfs_missing_extension_errors() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        // ACT
        let result = extend(
            base.path(),
            &[(
                "missing".to_string(),
                PathBuf::from("/nonexistent/extension-dir"),
            )],
            output.path(),
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
    async fn append_to_file_writes_data() {
        // ARRANGE
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(f.path(), b"hello").expect("write");

        // ACT
        append_to_file(f.path(), b" world").await.expect("append");

        // ASSERT
        assert_eq!(std::fs::read(f.path()).expect("read"), b"hello world");
    }

    #[tokio::test]
    async fn append_to_file_nonexistent_path_errors() {
        // ARRANGE / ACT
        let result = append_to_file(Path::new("/nonexistent/file"), b"data").await;

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

    #[test]
    fn create_writes_output_file() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init = tmp.path().join("init");
        std::fs::write(&init, b"#!/bin/sh\nexec /sbin/init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("mkdir rootfs");
        std::fs::write(rootfs.join("sbin/init"), b"init-binary").expect("write rootfs init");
        let output = tmp.path().join("initramfs.img");

        // ACT
        create(&create_config(&init, &rootfs), &output).expect("create");

        // ASSERT
        let data = std::fs::read(&output).expect("read output");
        assert!(!data.is_empty());
    }

    #[test]
    fn create_missing_output_parent_errors() {
        // ARRANGE
        let tmp = tempfile::tempdir().expect("tempdir");
        let init = tmp.path().join("init");
        std::fs::write(&init, b"#!/bin/sh\nexec /sbin/init").expect("write init");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).expect("mkdir rootfs");
        std::fs::write(rootfs.join("sbin/init"), b"init-binary").expect("write rootfs init");
        let output = tmp.path().join("missing/initramfs.img");

        // ACT
        let result = create(&create_config(&init, &rootfs), &output);

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
    fn write_compressed_extensions_archive_invalid_level_errors() {
        // ARRANGE / ACT
        let result = write_compressed_extensions_archive(Vec::new(), &extension_files(), i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn write_compressed_extensions_archive_cpio_errors() {
        // ARRANGE / ACT
        let result = write_compressed_extensions_archive_with(
            Vec::new(),
            &extension_files(),
            19,
            fail_cpio_archive::<Vec<u8>>,
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::CpioError(_)))
        );
    }

    #[test]
    fn write_compressed_extensions_archive_finish_errors() {
        // ARRANGE
        let files = extension_files();
        let mut encoder = zstd::Encoder::new(
            CountingFailingWriter {
                fail_on_call: usize::MAX,
                calls: 0,
            },
            19,
        )
        .expect("encoder");
        cpio::write_archive(&mut encoder, &files).expect("write archive");
        let fail_on_call = encoder.get_ref().calls + 1;

        // ACT
        let result = write_compressed_extensions_archive(
            CountingFailingWriter {
                fail_on_call,
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

    #[test]
    fn validate_compression_level_rejects_out_of_range_values() {
        // ARRANGE / ACT
        let result = validate_compression_level(i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }
}
