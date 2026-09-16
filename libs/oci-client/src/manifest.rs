//! Registry manifest fetching and pushing.

use hyper::body::Bytes;
use oci::media::OCI_MANIFEST_ACCEPT_HEADERS;
use oci::reference::Image;

use crate::client::Client;
use crate::error::{ClientError, Result};
use crate::http::{self, collect_body, get};

/// Build the manifest URL for a given image reference and tag or digest.
#[must_use]
pub fn build_url(image: &Image, reference: &str) -> String {
    format!(
        "{}://{}/v2/{}/manifests/{}",
        image.scheme(),
        image.registry,
        image.name,
        reference
    )
}

/// Fetch and return the raw manifest JSON from the registry.
///
/// # Errors
///
/// Returns an error when the request fails or the body is not UTF-8.
pub async fn fetch(client: &Client, manifest_ref: &str) -> Result<String> {
    let url = build_url(client.image(), manifest_ref);
    let resp = get(
        client.http(),
        &url,
        client.authorization(),
        OCI_MANIFEST_ACCEPT_HEADERS,
    )
    .await?;
    let body = collect_body(resp).await?;
    match core::str::from_utf8(&body) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => Err(ClientError::Network(format!(
            "Manifest response is not UTF-8: {error}"
        ))),
    }
}

/// Push a manifest to the registry via PUT.
///
/// # Errors
///
/// Returns an error when the request fails or the registry rejects the manifest.
pub async fn put(
    client: &Client,
    manifest_ref: &str,
    content_type: &str,
    body: Bytes,
) -> Result<()> {
    let url = build_url(client.image(), manifest_ref);
    http::put(
        client.http(),
        &url,
        client.authorization(),
        content_type,
        body,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_uses_registry_scheme_and_reference() {
        // ARRANGE
        let image = Image {
            registry: "127.0.0.1:5000".to_owned(),
            name: "repo/name".to_owned(),
            manifest_ref: "test".to_owned(),
        };

        // ACT / ASSERT
        assert_eq!(
            build_url(&image, "sha256:abc"),
            "http://127.0.0.1:5000/v2/repo/name/manifests/sha256:abc"
        );
    }
}
