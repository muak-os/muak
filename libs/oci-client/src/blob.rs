//! Registry blob operations per the distribution specification.

use hyper::body::Bytes;
use hyper::http::StatusCode;
use hyper::http::header::LOCATION;
use oci::reference::Image;

use crate::client::Client;
use crate::error::{ClientError, Result};
use crate::http;

/// Build the blob URL for a given image reference and digest.
#[must_use]
pub fn build_url(image: &Image, digest: &str) -> String {
    format!(
        "{}://{}/v2/{}/blobs/{}",
        image.scheme(),
        image.registry,
        image.name,
        digest
    )
}

/// Check whether the registry already holds a blob.
///
/// # Errors
///
/// Returns an error when the HEAD request fails or answers an unexpected status.
pub async fn exists(client: &Client, digest: &str) -> Result<bool> {
    let url = build_url(client.image(), digest);
    let response = http::head_any_status(client.http(), &url, client.authorization()).await?;
    match response.status() {
        StatusCode::OK => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        status => Err(ClientError::Push(format!(
            "blob HEAD returned HTTP {status} for {url}"
        ))),
    }
}

/// Upload a blob via the POST-then-PUT dance of the distribution spec.
///
/// # Errors
///
/// Returns an error when the upload session cannot start or the final PUT fails.
pub async fn upload(client: &Client, digest: &str, body: Bytes) -> Result<()> {
    let location = start(client).await?;
    finish(client, &location, digest, body).await
}

async fn start(client: &Client) -> Result<String> {
    let image = client.image();
    let url = format!(
        "{}://{}/v2/{}/blobs/uploads/",
        image.scheme(),
        image.registry,
        image.name
    );
    let response = http::post_any_status(client.http(), &url, client.authorization()).await?;
    if response.status() != StatusCode::ACCEPTED {
        return Err(ClientError::Push(format!(
            "blob upload start returned HTTP {} for {url}",
            response.status()
        )));
    }

    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ClientError::Push(format!("blob upload start carried no Location for {url}"))
        })?;

    Ok(resolve_location(image.scheme(), &image.registry, location))
}

async fn finish(client: &Client, location: &str, digest: &str, body: Bytes) -> Result<()> {
    let url = digest_url(location, digest);
    http::put(
        client.http(),
        &url,
        client.authorization(),
        "application/octet-stream",
        body,
    )
    .await?;

    Ok(())
}

fn digest_url(location: &str, digest: &str) -> String {
    let separator = if location.contains('?') { '&' } else { '?' };

    format!("{location}{separator}digest={digest}")
}

fn resolve_location(scheme: &str, registry: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_owned();
    }

    format!("{scheme}://{registry}/{}", location.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_uses_registry_scheme_and_digest() {
        // ARRANGE
        let image = Image {
            registry: "127.0.0.1:5000".to_owned(),
            name: "repo/name".to_owned(),
            manifest_ref: "test".to_owned(),
        };

        // ACT / ASSERT
        assert_eq!(
            build_url(&image, "sha256:abc"),
            "http://127.0.0.1:5000/v2/repo/name/blobs/sha256:abc"
        );
    }

    #[test]
    fn resolve_location_keeps_absolute_urls() {
        // ARRANGE
        let location = "https://other-host.example/v2/repo/blobs/uploads/u1";

        // ACT / ASSERT
        assert_eq!(
            resolve_location("https", "registry.example", location),
            location
        );
    }

    #[test]
    fn resolve_location_prefixes_absolute_and_relative_paths() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            resolve_location("http", "127.0.0.1:5000", "/v2/repo/blobs/uploads/u1"),
            "http://127.0.0.1:5000/v2/repo/blobs/uploads/u1"
        );
        assert_eq!(
            resolve_location("http", "127.0.0.1:5000", "v2/repo/blobs/uploads/u1"),
            "http://127.0.0.1:5000/v2/repo/blobs/uploads/u1"
        );
    }

    #[test]
    fn digest_url_appends_with_the_right_query_separator() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            digest_url(
                "https://r.example/v2/repo/blobs/uploads/u1?state=x",
                "sha256:abc"
            ),
            "https://r.example/v2/repo/blobs/uploads/u1?state=x&digest=sha256:abc"
        );
        assert_eq!(
            digest_url("https://r.example/v2/repo/blobs/uploads/u1", "sha256:abc"),
            "https://r.example/v2/repo/blobs/uploads/u1?digest=sha256:abc"
        );
    }
}
