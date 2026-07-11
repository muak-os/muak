//! OCI layer blob downloading and cache integration.

use super::cache::BlobCache;
use crate::digest::verify_blob_digest;
use crate::error::Result;
use crate::image::ImageReference;
use crate::registry::http::{HttpClient, collect_body, get};

/// Download a blob from the registry, verify its SHA-256 digest, and return the raw bytes.
pub(crate) async fn blob(
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

/// Download a blob, checking the local cache before hitting the network.
pub(crate) async fn cached(
    cache: &BlobCache,
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    if let Some(cached) = cache.get_blob(digest) {
        return Ok(cached);
    }
    let data = blob(client, image_ref, digest, token).await?;
    cache.put_blob(digest, &data);

    Ok(data)
}
