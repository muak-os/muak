use std::path::Path;

use flate2::read::GzDecoder;
use reqwest::Client;
use tar::Archive;

use crate::error::{ImagerError, Result};
use crate::image::ImageReference;
use crate::oci::http::build_authenticated_request;
use crate::oci::verify::verify_blob_digest;

/// Download blob bytes from the registry and verify its digest.
pub(crate) async fn download_blob(
    client: &Client,
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

    let response = build_authenticated_request(client, &blob_url, token, &[]).await?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ImagerError::NetworkError(format!("Failed to read blob response: {}", e)))?
        .to_vec();

    verify_blob_digest(&bytes, digest)?;

    Ok(bytes)
}

/// Extract tar archive from bytes to destination.
pub(crate) fn extract_archive(bytes: &[u8], dest: &Path) -> Result<()> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| ImagerError::LayerExtractionError(format!("Failed to extract tar: {}", e)))
}

