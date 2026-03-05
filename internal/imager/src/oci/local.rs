use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::{ImagerError, Result};
use crate::image::{OciIndex, OciManifest};
use crate::oci;

pub(crate) async fn extract_local_oci_layout(oci_dir: &Path) -> Result<PathBuf> {
    let oci_dir = oci_dir.to_path_buf();
    tokio::task::spawn_blocking(move || extract_local_oci_layout_blocking(&oci_dir))
        .await
        .map_err(|e| ImagerError::LayerExtractionError(e.to_string()))?
}

fn extract_local_oci_layout_blocking(oci_dir: &Path) -> Result<PathBuf> {
    let temp = oci::create_temp_dir("oci-")?;

    let index_path = oci_dir.join("index.json");
    let index_bytes = std::fs::read(&index_path).map_err(|e| ImagerError::ReadError {
        file: index_path.display().to_string(),
        source: e,
    })?;
    let index: OciIndex = serde_json::from_slice(&index_bytes)?;

    let manifest_digest = &index
        .manifests
        .first()
        .ok_or_else(|| ImagerError::InvalidOciFormat("No manifests in index".to_string()))?
        .digest;
    let manifest_blob = digest_to_blob_path(oci_dir, manifest_digest);
    let manifest_bytes = std::fs::read(&manifest_blob).map_err(|e| ImagerError::ReadError {
        file: manifest_blob.display().to_string(),
        source: e,
    })?;
    oci::verify::verify_local_digest(&manifest_bytes, manifest_digest, &manifest_blob)?;

    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)?;

    for layer in &manifest.layers {
        let layer_path = digest_to_blob_path(oci_dir, &layer.digest);
        let layer_bytes = std::fs::read(&layer_path).map_err(|e| ImagerError::ReadError {
            file: layer_path.display().to_string(),
            source: e,
        })?;
        oci::verify::verify_local_digest(&layer_bytes, &layer.digest, &layer_path)?;
        extract_tar_layer(&layer_bytes, temp.path())?;
    }

    Ok(temp.keep())
}

fn digest_to_blob_path(oci_dir: &Path, digest: &str) -> PathBuf {
    let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
    oci_dir.join("blobs").join("sha256").join(hash)
}

/// Extract a tar layer from in-memory bytes, detecting gzip compression.
fn extract_tar_layer(bytes: &[u8], dest: &Path) -> Result<()> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let decoder = GzDecoder::new(bytes);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest).map_err(|e| {
            ImagerError::LayerExtractionError(format!("Failed to extract gzipped layer: {}", e))
        })?;
    } else {
        let mut archive = Archive::new(bytes);
        archive.unpack(dest).map_err(|e| {
            ImagerError::LayerExtractionError(format!("Failed to extract layer: {}", e))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

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
