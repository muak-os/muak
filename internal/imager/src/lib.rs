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

use std::path::Path;

pub use error::{ImagerError, Result};

// Import OCI functions directly from submodules
use oci::local::extract_local_oci_layout;
use oci::remote::{pull_to_dir, pull_to_temp};

/// Pull an OCI image and extract it to a directory.
///
/// This downloads an OCI image from a registry (or extracts from a local OCI layout)
/// and extracts all layers to the output directory.
///
/// # Arguments
///
/// * `reference` - Image reference (e.g., "alpine:latest", "ghcr.io/org/image:v1")
/// * `output` - Output directory path
///
/// # Errors
///
/// Returns an `ImagerError` if:
/// - The image cannot be downloaded or accessed
/// - The OCI structure is invalid
/// - Layer extraction fails
/// - IO operations fail
///
/// # Example
///
/// ```no_run
/// # use std::path::Path;
/// imager::pull_image("alpine:latest", Path::new("/tmp/alpine"))?;
/// # Ok::<(), imager::error::ImagerError>(())
/// ```
pub fn pull_image(reference: &str, output: &Path) -> Result<()> {
    std::fs::create_dir_all(output)?;
    pull_to_dir(reference, output)
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
/// let extensions = vec!["alpine-base".to_string(), "ghcr.io/org/ext:v1".to_string()];
/// let extension_archive = imager::build_extensions_archive(&extensions)?;
/// # Ok::<(), imager::error::ImagerError>(())
/// ```
pub fn build_extensions_archive(extensions: &[String]) -> Result<Vec<u8>> {
    let files = process_extensions(extensions)?;
    let cpio_data = cpio::create_archive(&files)?;
    let compressed = zstd::encode_all(&cpio_data[..], 19)
        .map_err(|e| ImagerError::CpioError(format!("Compression failed: {}", e)))?;
    Ok(compressed)
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
/// let extensions = vec!["alpine-base".to_string(), "ghcr.io/org/ext:v1".to_string()];
/// imager::build_initramfs(
///     Path::new("base-initramfs.img"),
///     &extensions,
///     Path::new("custom-initramfs.img")
/// )?;
/// # Ok::<(), imager::error::ImagerError>(())
/// ```
pub fn build_initramfs(base: &Path, extensions: &[String], output: &Path) -> Result<()> {
    std::fs::copy(base, output).map_err(|e| ImagerError::ReadError {
        file: base.display().to_string(),
        source: e,
    })?;

    if extensions.is_empty() {
        return Ok(());
    }

    let extension_archive = build_extensions_archive(extensions)?;
    let mut output_file = std::fs::OpenOptions::new()
        .append(true)
        .open(output)
        .map_err(|e| ImagerError::WriteError {
            file: output.display().to_string(),
            source: e,
        })?;

    std::io::Write::write_all(&mut output_file, &extension_archive).map_err(|e| {
        ImagerError::WriteError {
            file: output.display().to_string(),
            source: e,
        }
    })?;

    Ok(())
}

/// Process extensions and create squashfs archives for each.
///
/// This is an internal helper function that:
/// - Handles both local OCI layouts and remote OCI images
/// - Extracts/pulls each extension
/// - Creates a squashfs archive from the extracted content
///
/// Returns a vector of (path, data) tuples ready for CPIO packaging.
fn process_extensions(extensions: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();

    for ext in extensions {
        let (name, temp_dir) = if Path::new(ext).exists() {
            let name = Path::new(ext)
                .file_name()
                .ok_or(ImagerError::InvalidOciFormat(
                    "extension path has no file name".to_string(),
                ))?
                .to_string_lossy()
                .to_string();
            let dir = extract_local_oci_layout(Path::new(ext))?;
            (name, dir)
        } else {
            let name = image::ImageReference::parse(ext).image_name();
            let dir = pull_to_temp(ext)?;
            (name, dir)
        };
        let temp_path = temp_dir.as_path();
        let sqsh_data = squashfs::create_at(temp_path)?;
        files.push((format!("extensions/{}.sqsh", name), sqsh_data));
    }

    Ok(files)
}
