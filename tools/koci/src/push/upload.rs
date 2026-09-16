//! OCI blob upload per the distribution registry specification.

use hyper::body::Bytes;
use hyper::http::StatusCode;
use hyper::http::header::LOCATION;

use crate::error::{KociError, Result};
use crate::registry::http;
use crate::registry::session::Session;

/// Upload a blob unless the registry already holds it.
///
/// # Errors
///
/// Returns an error when the existence check, the session start, or the final PUT fails.
pub(crate) async fn blob(session: &Session, digest: &str, body: Bytes) -> Result<()> {
    if exists(session, digest).await? {
        eprintln!("Blob {digest} already in registry; skipping upload");

        return Ok(());
    }

    let location = start(session).await?;
    finish(session, &location, digest, body).await
}

async fn exists(session: &Session, digest: &str) -> Result<bool> {
    let url = format!(
        "{}://{}/v2/{}/blobs/{}",
        session.image.scheme(),
        session.image.registry,
        session.image.name,
        digest
    );
    let response = http::head_any_status(&session.client, &url, session.authorization()).await?;
    match response.status() {
        StatusCode::OK => Ok(true),
        StatusCode::NOT_FOUND => Ok(false),
        status => Err(push_error(format!(
            "blob HEAD returned HTTP {status} for {url}"
        ))),
    }
}

async fn start(session: &Session) -> Result<String> {
    let url = format!(
        "{}://{}/v2/{}/blobs/uploads/",
        session.image.scheme(),
        session.image.registry,
        session.image.name
    );
    let response = http::post_any_status(&session.client, &url, session.authorization()).await?;
    if response.status() != StatusCode::ACCEPTED {
        return Err(push_error(format!(
            "blob upload start returned HTTP {} for {url}",
            response.status()
        )));
    }

    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| push_error(format!("blob upload start carried no Location for {url}")))?;

    Ok(resolve_location(
        session.image.scheme(),
        &session.image.registry,
        location,
    ))
}

async fn finish(session: &Session, location: &str, digest: &str, body: Bytes) -> Result<()> {
    let url = digest_url(location, digest);
    http::put(
        &session.client,
        &url,
        session.authorization(),
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

fn push_error(details: impl core::fmt::Display) -> KociError {
    KociError::PushError(details.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
