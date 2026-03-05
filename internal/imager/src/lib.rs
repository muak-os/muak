//! Imager library for OCI image management and initramfs generation.
//!
//! This library provides functionality to:
//! - Pull and extract OCI images from registries or local directories
//! - Build custom initramfs by merging base initramfs with extensions as overlays

mod cpio;
mod image;
mod oci;
mod squashfs;

pub mod error;

use std::path::{Path, PathBuf};

pub use error::{ImagerError, Result};
// Import OCI functions directly from submodules
use oci::local::extract_local_oci_layout;
use oci::remote::{pull_to_dir, pull_to_temp};

/// Maximum number of extensions to process concurrently.
const MAX_CONCURRENT_EXTENSIONS: usize = 8;

/// Pull an OCI image and extract it to a directory.
///
/// This downloads an OCI image from a registry (or extracts from a local OCI layout)
/// and extracts all layers to the output directory.
///
/// # Arguments
///
/// * `reference` - Image reference (e.g., "alpine:latest", "ghcr.io/org/image:v1")
/// * `output` - Output directory path
/// * `pubkey_pem` - Optional PEM-encoded ECDSA P-256 cosign public key. When `None`,
///   the key embedded at compile time (`cosign.pub` at the repository root) is used.
///
/// # Errors
///
/// Returns an `ImagerError` if:
/// - The image cannot be downloaded or accessed
/// - The OCI structure is invalid
/// - Cosign signature verification fails
/// - Layer extraction fails
/// - IO operations fail
///
/// # Example
///
/// ```no_run
/// # use std::path::Path;
/// # async fn example() -> imager::Result<()> {
/// // Use the default embedded key
/// imager::pull_image("alpine:latest", Path::new("/tmp/alpine"), None).await?;
///
/// // Use a custom key
/// let key = std::fs::read_to_string("/path/to/custom.pub").unwrap();
/// imager::pull_image("registry.local/my-image:latest", Path::new("/tmp/out"), Some(&key)).await?;
/// # Ok(())
/// # }
/// ```
pub async fn pull_image(reference: &str, output: &Path, pubkey_pem: Option<&str>) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    pull_to_dir(reference, output, pubkey_pem).await
}

/// Build a compressed CPIO archive containing squashfs files for each extension.
///
/// This function processes each extension (pulling from registry or local directory),
/// creates a squashfs archive for each, packages them into a CPIO archive, and
/// compresses it with zstd. The result is ready to be appended to a base initramfs.
///
/// The extensions are made available in the initramfs at `/extensions/{name}.sqsh`.
///
/// # Arguments
///
/// * `extensions` - Slice of extension references (local paths or OCI image references)
///
/// # Errors
///
/// Returns an `ImagerError` if:
/// - Extension processing fails (download, extraction, or squashfs creation)
/// - CPIO archive creation fails
/// - Compression fails
///
/// # Example
///
/// ```no_run
/// # async fn example() -> imager::Result<()> {
/// let extensions = vec!["alpine-base".to_string(), "ghcr.io/org/ext:v1".to_string()];
/// let extension_archive = imager::build_extensions_archive(&extensions).await?;
/// # Ok(())
/// # }
/// ```
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
///
/// This function:
/// 1. Copies the base initramfs to the output location
/// 2. Processes each extension (pulling from registry or local directory)
/// 3. Creates a squashfs archive for each extension
/// 4. Packages all extensions into a CPIO archive
/// 5. Compresses the CPIO archive with zstd
/// 6. Appends it to the output initramfs
///
/// The extensions are made available in the initramfs at `/extensions/{name}.sqsh`.
///
/// # Arguments
///
/// * `base` - Path to the base initramfs file
/// * `extensions` - Slice of extension references (local paths or OCI image references)
/// * `output` - Path where the final initramfs will be written
///
/// # Errors
///
/// Returns an `ImagerError` if:
/// - The base file cannot be read or written
/// - Extension processing fails (download, extraction, or squashfs creation)
/// - CPIO archive creation fails
/// - Compression fails
///
/// # Example
///
/// ```no_run
/// # use std::path::Path;
/// # async fn example() -> imager::Result<()> {
/// let extensions = vec!["alpine-base".to_string(), "ghcr.io/org/ext:v1".to_string()];
/// imager::build_initramfs(
///     Path::new("base-initramfs.img"),
///     &extensions,
///     Path::new("custom-initramfs.img")
/// ).await?;
/// # Ok(())
/// # }
/// ```
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
///
/// Returns a vector of (path, data) tuples ready for CPIO packaging.
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
    let (name, temp_dir) = if Path::new(&ext).exists() {
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
