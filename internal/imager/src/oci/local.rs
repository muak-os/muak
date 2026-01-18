use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use tar::Archive;

use crate::error::{ImagerError, Result};
use crate::image::{OciIndex, OciManifest};
use crate::oci::remote::create_temp_dir;

pub(crate) fn extract_local_oci_layout(oci_dir: &Path) -> Result<PathBuf> {
    let temp = create_temp_dir("oci-")?;

    let index_path = oci_dir.join("index.json");
    let index: OciIndex =
        serde_json::from_reader(BufReader::new(File::open(&index_path).map_err(|e| {
            ImagerError::ReadError {
                file: index_path.display().to_string(),
                source: e,
            }
        })?))?;

    let manifest_digest = &index
        .manifests
        .first()
        .ok_or_else(|| ImagerError::InvalidOciFormat("No manifests in index".to_string()))?
        .digest;
    let manifest_blob = digest_to_blob_path(oci_dir, manifest_digest);
    let manifest: OciManifest =
        serde_json::from_reader(BufReader::new(File::open(&manifest_blob).map_err(|e| {
            ImagerError::ReadError {
                file: manifest_blob.display().to_string(),
                source: e,
            }
        })?))?;

    for layer in &manifest.layers {
        let layer_path = digest_to_blob_path(oci_dir, &layer.digest);
        extract_tar_layer(&layer_path, temp.path())?;
    }

    Ok(temp.keep())
}

fn digest_to_blob_path(oci_dir: &Path, digest: &str) -> PathBuf {
    let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
    oci_dir.join("blobs").join("sha256").join(hash)
}

fn extract_tar_layer(layer_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(layer_path).map_err(|e| ImagerError::ReadError {
        file: layer_path.display().to_string(),
        source: e,
    })?;
    let mut reader = BufReader::new(file);

    let mut magic = [0u8; 2];
    reader.read_exact(&mut magic).map_err(|e| {
        ImagerError::LayerExtractionError(format!("Failed to read layer magic: {}", e))
    })?;
    reader
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| ImagerError::LayerExtractionError(format!("Seek failed: {}", e)))?;

    if magic == [0x1f, 0x8b] {
        let decoder = GzDecoder::new(reader);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest).map_err(|e| {
            ImagerError::LayerExtractionError(format!("Failed to extract gzipped layer: {}", e))
        })?;
    } else {
        let mut archive = Archive::new(reader);
        archive.unpack(dest).map_err(|e| {
            ImagerError::LayerExtractionError(format!("Failed to extract layer: {}", e))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_digest_to_blob_path_with_sha256_prefix() {
        let oci_dir = Path::new("/tmp/oci");
        let digest = "sha256:abcd1234";
        let expected = oci_dir.join("blobs").join("sha256").join("abcd1234");
        assert_eq!(digest_to_blob_path(oci_dir, digest), expected);
    }

    #[test]
    fn test_digest_to_blob_path_without_prefix() {
        let oci_dir = Path::new("/tmp/oci");
        let digest = "abcd1234";
        let expected = oci_dir.join("blobs").join("sha256").join("abcd1234");
        assert_eq!(digest_to_blob_path(oci_dir, digest), expected);
    }

    #[test]
    fn test_digest_to_blob_path_empty_digest() {
        let oci_dir = Path::new("/tmp/oci");
        let digest = "";
        let expected = oci_dir.join("blobs").join("sha256").join("");
        assert_eq!(digest_to_blob_path(oci_dir, digest), expected);
    }

    #[test]
    fn test_digest_to_blob_path_long_digest() {
        let oci_dir = Path::new("/tmp/oci");
        let digest = "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let expected = oci_dir
            .join("blobs")
            .join("sha256")
            .join("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890");
        assert_eq!(digest_to_blob_path(oci_dir, digest), expected);
    }
}
