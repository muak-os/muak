//! Ramune: initramfs builder from pre-extracted extension directories.

mod cpio;
mod erofs;
mod extension;

pub mod error;

use std::path::{Path, PathBuf};

pub use error::{RamuneError, Result};

/// Builds an initramfs by appending a zstd-compressed EROFS extension archive to a base image.
pub async fn build_initramfs(base: &Path, extensions: &[PathBuf], output: &Path) -> Result<()> {
    tokio::fs::copy(base, output)
        .await
        .map_err(|e| RamuneError::ReadError {
            file: base.display().to_string(),
            source: e,
        })?;

    if extensions.is_empty() {
        return Ok(());
    }

    let archive = build_extensions_archive(extensions).await?;
    append_to_file(output, &archive).await
}

/// Builds a zstd-compressed CPIO archive of EROFS images for each extension directory.
async fn build_extensions_archive(extensions: &[PathBuf]) -> Result<Vec<u8>> {
    let files = extension::process_all(extensions).await?;
    tokio::task::spawn_blocking(move || {
        let cpio_data = cpio::create_archive(&files)?;
        zstd::encode_all(&cpio_data[..], 19)
            .map_err(|e| RamuneError::CpioError(format!("Compression failed: {e}")))
    })
    .await
    .map_err(|e| RamuneError::CpioError(e.to_string()))?
}

/// Appends data to the file at `path`.
async fn append_to_file(path: &Path, data: &[u8]) -> Result<()> {
    let path = path.to_path_buf();
    let data = data.to_vec();
    tokio::task::spawn_blocking(move || {
        let file_str = path.display().to_string();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| RamuneError::WriteError {
                file: file_str.clone(),
                source: e,
            })?;
        std::io::Write::write_all(&mut file, &data).map_err(|e| RamuneError::WriteError {
            file: file_str,
            source: e,
        })
    })
    .await
    .map_err(|e| RamuneError::WriteError {
        file: "initramfs".to_string(),
        source: std::io::Error::other(e),
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_initramfs_no_extensions() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base-initramfs-content").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        // ACT
        build_initramfs(base.path(), &[], output.path())
            .await
            .expect("build_initramfs");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert_eq!(content, b"base-initramfs-content");
    }

    #[tokio::test]
    async fn build_initramfs_with_extensions() {
        // ARRANGE
        let base = tempfile::NamedTempFile::new().expect("base tempfile");
        std::fs::write(base.path(), b"base").expect("write base");
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        let ext_dir = tempfile::TempDir::new().expect("ext dir");
        std::fs::write(ext_dir.path().join("hello.txt"), b"world").expect("write ext file");

        // ACT
        build_initramfs(base.path(), &[ext_dir.path().to_path_buf()], output.path())
            .await
            .expect("build_initramfs");

        // ASSERT
        let content = std::fs::read(output.path()).expect("read output");
        assert!(content.len() > 4);
        assert!(content.starts_with(b"base"));
    }

    #[tokio::test]
    async fn build_initramfs_missing_base() {
        // ARRANGE
        let output = tempfile::NamedTempFile::new().expect("output tempfile");

        // ACT
        let result = build_initramfs(Path::new("/nonexistent/base.img"), &[], output.path()).await;

        // ASSERT
        assert!(matches!(result, Err(RamuneError::ReadError { .. })));
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
        assert!(matches!(result, Err(RamuneError::WriteError { .. })));
    }
}
