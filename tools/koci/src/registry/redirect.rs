//! Redirect following for registry GET requests.

use hyper::Response;
use hyper::body::Incoming;

use crate::error::{KociError, Result};
use crate::registry::http::{HttpClient, get_any_status};

/// Maximum number of redirects followed for a single request.
const MAX_REDIRECTS: usize = 5;

/// Execute an authorized GET, following up to [`MAX_REDIRECTS`] redirects.
///
/// # Errors
///
/// Returns an error when a request fails, a redirect cannot be resolved, or
/// too many redirects are followed.
pub(crate) async fn follow(
    client: &HttpClient,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    let mut current = url.to_owned();
    let mut authorization = authorization;

    for _ in 0..MAX_REDIRECTS {
        let response = get_any_status(client, &current, authorization, accept_headers).await?;

        if !is_redirect(response.status().as_u16()) {
            return Ok(response);
        }

        let Some(location) = header_location(&response) else {
            break;
        };

        let next = resolve(&current, location).ok_or_else(|| {
            KociError::DownloadError(format!(
                "Unresolvable redirect Location `{location}` for URL: {url}"
            ))
        })?;

        if !same_host(&current, &next) {
            authorization = None;
        }
        current = next;
    }

    Err(KociError::DownloadError(format!(
        "Failed to follow redirects for URL: {url}"
    )))
}

/// Whether the status code is a redirect that should be followed.
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Extracts the `Location` header value as UTF-8, if present.
fn header_location(response: &Response<Incoming>) -> Option<&str> {
    response
        .headers()
        .get(hyper::header::LOCATION)
        .and_then(|value| value.to_str().ok())
}

/// Resolves a `Location` value against the request URL.
fn resolve(current: &str, location: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_owned());
    }

    let (scheme, _) = current.split_once("://")?;
    let authority = host_of(current)?;

    if location.starts_with('/') {
        return Some(format!("{scheme}://{authority}{location}"));
    }

    None
}

/// Whether two URLs point at the same authority (host and port).
fn same_host(left: &str, right: &str) -> bool {
    match (host_of(left), host_of(right)) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

/// Returns the authority (host[:port]) of a URL.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;

    Some(rest.split(['/', '?', '#']).next().unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_redirect_accepts_followable_statuses_only() {
        // ARRANGE
        let follow = [301, 302, 303, 307, 308];
        let stop = [200, 201, 400, 401, 404, 500];

        // ACT / ASSERT
        for status in follow {
            assert!(is_redirect(status), "{status} must be followed");
        }
        for status in stop {
            assert!(!is_redirect(status), "{status} must not be followed");
        }
    }

    #[test]
    fn resolve_keeps_absolute_urls() {
        // ARRANGE
        let current = "https://ghcr.io/v2/muak-os/stub/blobs/sha256:abc";

        // ACT
        let resolved = resolve(current, "https://storage.example.com/blob?sig=token");

        // ASSERT
        assert_eq!(
            resolved.as_deref(),
            Some("https://storage.example.com/blob?sig=token")
        );
    }

    #[test]
    fn resolve_prefixes_absolute_paths_with_the_authority() {
        // ARRANGE
        let current = "https://ghcr.io/v2/muak-os/stub/blobs/sha256:abc";

        // ACT
        let resolved = resolve(current, "/v2/other/blobs/sha256:def");

        // ASSERT
        assert_eq!(
            resolved.as_deref(),
            Some("https://ghcr.io/v2/other/blobs/sha256:def")
        );
    }

    #[test]
    fn resolve_rejects_relative_paths_and_urlless_currents() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(resolve("https://ghcr.io/v2/x", "relative/blob"), None);
        assert_eq!(resolve("not a url", "/v2/x"), None);
    }

    #[test]
    fn same_host_compares_authorities_case_insensitively() {
        // ARRANGE / ACT / ASSERT
        assert!(same_host("https://GHCR.io/v2/x", "https://ghcr.io/v2/y"));
        assert!(!same_host(
            "https://ghcr.io/v2/x",
            "https://ghcr.io:443/v2/x"
        ));
        assert!(!same_host(
            "https://ghcr.io/v2/x",
            "https://storage.example.com/blob"
        ));
        assert!(!same_host("not a url", "https://ghcr.io/v2/x"));
    }

    #[test]
    fn host_of_extracts_authority_and_strips_path_and_query() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(host_of("https://ghcr.io/v2/x?n=1"), Some("ghcr.io"));
        assert_eq!(
            host_of("http://localhost:5000/v2/y"),
            Some("localhost:5000")
        );
        assert_eq!(host_of("http://localhost:5000"), Some("localhost:5000"));
        assert_eq!(host_of("nonsense"), None);
    }
}
