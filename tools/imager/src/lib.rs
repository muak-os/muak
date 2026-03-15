//! Imager library for OCI image management and initramfs generation.

mod cpio;
mod image;
mod oci;
mod squashfs;

pub mod error;

use std::path::{Path, PathBuf};

pub use error::{ImagerError, Result};
use oci::local::extract_local_oci_layout;
use oci::remote::{pull_to_dir, pull_to_temp};
use oci::sign::sign_manifest;

/// Maximum number of extensions to process concurrently.
const MAX_CONCURRENT_EXTENSIONS: usize = 8;

/// Pull an OCI image and extract it to a directory.
pub async fn pull_image(reference: &str, output: &Path, pubkey_pem: Option<&str>) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    pull_to_dir(reference, output, pubkey_pem).await
}

/// Sign an OCI image manifest in the registry.
pub async fn sign_image(reference: &str, privkey_pem: &str) -> Result<()> {
    sign_manifest(reference, privkey_pem).await
}

/// Build a compressed CPIO archive containing squashfs files for each extension.
pub async fn build_extensions_archive(extensions: &[String]) -> Result<Vec<u8>> {
    let files = process_extensions(extensions).await?;
    tokio::task::spawn_blocking(move || {
        let cpio_data = cpio::create_archive(&files)?;
        zstd::encode_all(&cpio_data[..], 19)
            .map_err(|e| ImagerError::CpioError(format!("Compression failed: {}", e)))
    })
    .await
    .map_err(|e| ImagerError::CpioError(e.to_string()))?
}

/// Build a custom initramfs by merging a base with extensions.
pub async fn build_initramfs(base: &Path, extensions: &[String], output: &Path) -> Result<()> {
    tokio::fs::copy(base, output)
        .await
        .map_err(|e| ImagerError::ReadError {
            file: base.display().to_string(),
            source: e,
        })?;

    if extensions.is_empty() {
        return Ok(());
    }

    let extension_archive = build_extensions_archive(extensions).await?;
    append_to_file(output.to_path_buf(), extension_archive).await
}

/// Append data to a file using a blocking task.
async fn append_to_file(path: PathBuf, data: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| ImagerError::WriteError {
                file: path.display().to_string(),
                source: e,
            })?;
        std::io::Write::write_all(&mut file, &data).map_err(|e| ImagerError::WriteError {
            file: path.display().to_string(),
            source: e,
        })
    })
    .await
    .map_err(|e| ImagerError::WriteError {
        file: "initramfs".to_string(),
        source: std::io::Error::other(e),
    })?
}

/// Process extensions concurrently and create squashfs archives for each.
async fn process_extensions(extensions: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut join_set = tokio::task::JoinSet::new();
    let mut iter = extensions.iter().cloned();
    let mut files = Vec::with_capacity(extensions.len());

    spawn_extension_batch(&mut join_set, &mut iter);

    while let Some(result) = join_set.join_next().await {
        files.push(result.map_err(|e| ImagerError::SquashfsError(e.to_string()))??);
        spawn_extension_batch(&mut join_set, &mut iter);
    }

    Ok(files)
}

/// Spawn extension processing tasks until the concurrency limit is reached.
fn spawn_extension_batch(
    join_set: &mut tokio::task::JoinSet<Result<(String, Vec<u8>)>>,
    iter: &mut impl Iterator<Item = String>,
) {
    while join_set.len() < MAX_CONCURRENT_EXTENSIONS {
        let Some(ext) = iter.next() else {
            return;
        };
        join_set.spawn(process_single_extension(ext));
    }
}

/// Process a single extension: pull/extract and create squashfs.
async fn process_single_extension(ext: String) -> Result<(String, Vec<u8>)> {
    let (name, temp_dir) = if is_local_path(&ext) {
        let name = Path::new(&ext)
            .file_name()
            .ok_or(ImagerError::InvalidOciFormat(
                "extension path has no file name".to_string(),
            ))?
            .to_string_lossy()
            .to_string();
        let dir = extract_local_oci_layout(Path::new(&ext)).await?;
        (name, dir)
    } else {
        let name = image::ImageReference::parse(&ext).image_name();
        let dir = pull_to_temp(&ext, None).await?;
        (name, dir)
    };

    let sqsh_data = tokio::task::spawn_blocking(move || squashfs::create_at(&temp_dir))
        .await
        .map_err(|e| ImagerError::SquashfsError(e.to_string()))??;

    Ok((format!("extensions/{}.sqsh", name), sqsh_data))
}

/// Returns true if the extension string refers to a local filesystem path.
fn is_local_path(ext: &str) -> bool {
    let p = Path::new(ext);
    p.is_absolute() || ext.starts_with("./") || ext.starts_with("../") || p.exists()
}
