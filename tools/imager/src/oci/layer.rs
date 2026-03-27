//! OCI layer download, digest verification, and archive extraction.

use std::path::Path;

use flate2::read::GzDecoder;
use tar::Archive;

use crate::error::{ImagerError, Result};
use crate::image::ImageReference;
use crate::oci::http::{HttpClient, collect_body, get};
use crate::oci::verify::verify_blob_digest;

/// Download a blob from the registry, verify its SHA-256 digest, and return the raw bytes.
pub(crate) async fn download_blob(
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let blob_url = format!(
        "{}://{}/v2/{}/blobs/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        digest
    );

    let resp = get(client, &blob_url, token, &[]).await?;
    let bytes = collect_body(resp).await?.to_vec();
    verify_blob_digest(&bytes, digest)?;
    Ok(bytes)
}

/// Extract a gzip-compressed tar archive from `bytes` into `dest`.
pub(crate) fn extract_archive(bytes: &[u8], dest: &Path) -> Result<()> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| ImagerError::LayerExtractionError(format!("Failed to extract tar: {}", e)))
}
