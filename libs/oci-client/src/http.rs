//! Shared HTTP/HTTPS request execution for OCI registry communication.

use core::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::http::StatusCode;
use hyper::http::header::RANGE;
use hyper::{Request, Response};
use oci::digest::Verifier;
use tokio::time::timeout;

use crate::error::{ClientError, Result};
use crate::redirect;
use crate::request;
use crate::retry;
use crate::transport::Transport;

/// Deadline for a request to reach its response head.
const HTTP_TIMEOUT: Duration = Duration::from_mins(1);

/// Per-frame read deadline for streamed response bodies.
const BODY_TIMEOUT: Duration = Duration::from_mins(1);

/// A byte range for resumable downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// First byte of the range, inclusive.
    pub start: u64,
    /// Last byte of the range, inclusive; `None` runs to the end of the body.
    pub end: Option<u64>,
}

impl Range {
    /// A range from `start` to the end of the body.
    #[must_use]
    pub fn from_start(start: u64) -> Self {
        Self { start, end: None }
    }

    /// The `Range` header value for this range.
    #[must_use]
    pub(crate) fn header_value(&self) -> String {
        match self.end {
            Some(end) => format!("bytes={}-{}", self.start, end),
            None => format!("bytes={}-", self.start),
        }
    }
}

/// Execute an authorized GET, following redirects and retrying transient failures.
///
/// # Errors
///
/// Returns an error when a request fails or the registry answers non-2xx.
pub async fn get(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    get_with_range(client, url, authorization, accept_headers, None).await
}

/// Execute an authorized ranged GET for resumable downloads.
///
/// # Errors
///
/// Returns an error when a request fails or the registry answers non-2xx.
pub async fn get_range(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
    range: &Range,
) -> Result<Response<Incoming>> {
    get_with_range(client, url, authorization, accept_headers, Some(range)).await
}

/// Execute a GET and return the response whatever its status.
///
/// # Errors
///
/// Returns an error when the request cannot be built or sent.
pub async fn get_any_status(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
    range: Option<&Range>,
) -> Result<Response<Incoming>> {
    let req = request::get_request(url, authorization, accept_headers, range)?;

    send(client, url, req).await
}

/// Execute a HEAD and return the response whatever its status, retrying
/// transient failures.
///
/// # Errors
///
/// Returns an error when the request cannot be built or sent.
pub async fn head_any_status(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
) -> Result<Response<Incoming>> {
    let policy = retry::Policy::default();
    let mut attempt = 0;
    loop {
        let req = request::head_request(url, authorization)?;
        match send(client, url, req).await {
            Ok(response) => return Ok(response),
            Err(error) => match retry::next_retry(&policy, &error, attempt) {
                Some(delay) => {
                    tokio::time::sleep(delay).await;
                    attempt = attempt.saturating_add(1);
                }
                None => return Err(error),
            },
        }
    }
}

/// Execute a POST with an empty body and return the response whatever its status.
///
/// # Errors
///
/// Returns an error when the request cannot be built or sent.
pub async fn post_any_status(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
) -> Result<Response<Incoming>> {
    let req = request::post_request(url, authorization)?;

    send(client, url, req).await
}

/// Execute an authorized PUT with a raw body, returning the response on 2xx.
///
/// # Errors
///
/// Returns an error when the request fails or the registry answers non-2xx.
pub async fn put(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Response<Incoming>> {
    let req = request::put_request(url, authorization, content_type, body)?;
    let response = send(client, url, req).await?;

    ensure_success(url, response)
}

/// Execute an authorized PATCH with a raw body, returning the response on 2xx.
///
/// # Errors
///
/// Returns an error when the request fails or the registry answers non-2xx.
pub async fn patch(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    content_range: &str,
    content_length: usize,
    body: Bytes,
) -> Result<Response<Incoming>> {
    let req = request::patch_request(
        url,
        authorization,
        content_type,
        content_range,
        content_length,
        body,
    )?;
    let response = send(client, url, req).await?;

    ensure_success(url, response)
}

/// The `Range` response header as an inclusive `(start, end)` byte pair.
///
/// # Errors
///
/// Returns an error when the header is missing or malformed.
pub fn confirmed_range(response: &Response<Incoming>) -> Result<(u64, u64)> {
    let value = response
        .headers()
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ClientError::Push("chunk upload response carried no Range header".to_owned())
        })?;
    let stripped = value
        .strip_prefix("0-")
        .ok_or_else(|| ClientError::Push(format!("bad upload Range header: {value}")))?;
    let end = stripped
        .parse::<u64>()
        .map_err(|error| ClientError::Push(format!("bad upload Range header: {value}: {error}")))?;

    Ok((0, end))
}

/// Whether the response is a partial body (`206 Partial Content`).
#[must_use]
pub fn is_partial<B>(response: &Response<B>) -> bool {
    response.status() == StatusCode::PARTIAL_CONTENT
}

/// Fully collect an HTTP response body into [`Bytes`].
///
/// # Errors
///
/// Returns an error when reading the body times out or fails.
pub async fn collect_body(resp: Response<Incoming>) -> Result<Bytes> {
    timeout(HTTP_TIMEOUT, resp.into_body().collect())
        .await
        .map_err(|error| {
            ClientError::Network(format!(
                "HTTP response body timed out after {HTTP_TIMEOUT:?}: {error}"
            ))
        })?
        .map(http_body_util::Collected::to_bytes)
        .map_err(|error| ClientError::Network(format!("Failed to read response body: {error}")))
}

/// Stream an HTTP response body into a writer while computing a digest.
///
/// # Errors
///
/// Returns an error when reading the body times out, fails, or the sink rejects a write.
pub async fn stream_body_to_sink<W: std::io::Write>(
    resp: Response<Incoming>,
    sink: &mut W,
    digest: &mut Verifier,
) -> Result<()> {
    let mut body = resp.into_body();

    while let Some(frame) = timeout(BODY_TIMEOUT, body.frame()).await.map_err(|error| {
        ClientError::Network(format!(
            "HTTP response body timed out after {BODY_TIMEOUT:?}: {error}"
        ))
    })? {
        let frame = frame.map_err(|error| {
            ClientError::Network(format!("Failed to read response body: {error}"))
        })?;

        if let Some(data) = frame.data_ref() {
            digest.update(data);
            write_frame(sink, data)?;
        }
    }

    Ok(())
}

fn write_frame<W: std::io::Write>(sink: &mut W, data: &[u8]) -> Result<()> {
    sink.write_all(data)
        .map_err(|error| ClientError::Network(format!("Failed to write response body: {error}")))
}

async fn get_with_range(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
    range: Option<&Range>,
) -> Result<Response<Incoming>> {
    let policy = retry::Policy::default();
    let mut attempt = 0;
    loop {
        let result = match redirect::follow(client, url, authorization, accept_headers, range).await
        {
            Ok(response) => ensure_success(url, response),
            Err(error) => Err(error),
        };
        match result {
            Ok(response) => return Ok(response),
            Err(error) => match retry::next_retry(&policy, &error, attempt) {
                Some(delay) => {
                    tokio::time::sleep(delay).await;
                    attempt = attempt.saturating_add(1);
                }
                None => return Err(error),
            },
        }
    }
}

async fn send(
    client: &Transport,
    url: &str,
    req: Request<Full<Bytes>>,
) -> Result<Response<Incoming>> {
    timeout(HTTP_TIMEOUT, client.request(req))
        .await
        .map_err(|error| {
            ClientError::Network(format!(
                "HTTP request timed out after {HTTP_TIMEOUT:?} for URL: {url}: {error}"
            ))
        })?
        .map_err(|error| ClientError::Network(format!("HTTP request failed: {error}")))
}

fn ensure_success(url: &str, response: Response<Incoming>) -> Result<Response<Incoming>> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(ClientError::Status {
            status: response.status().as_u16(),
            url: url.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = crate::transport::build_client();

        // ACT
        let error = get(&client, "http://127.0.0.1:5000/has space", None, &[])
            .await
            .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[tokio::test]
    async fn put_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = crate::transport::build_client();

        // ACT
        let error = put(
            &client,
            "http://127.0.0.1:5000/has space",
            Some("token"),
            "application/json",
            Bytes::from_static(b"{}"),
        )
        .await
        .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[tokio::test]
    async fn head_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = crate::transport::build_client();

        // ACT
        let error = head_any_status(&client, "http://127.0.0.1:5000/has space", None)
            .await
            .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[tokio::test]
    async fn post_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = crate::transport::build_client();

        // ACT
        let error = post_any_status(&client, "http://127.0.0.1:5000/has space", None)
            .await
            .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[tokio::test]
    async fn get_reports_connection_failures() {
        // ARRANGE
        let client = crate::transport::build_client();

        // ACT
        let error = get(
            &client,
            "http://127.0.0.1:9/v2/repo/manifests/test",
            None,
            &[],
        )
        .await
        .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[test]
    fn range_headers_cover_open_and_closed_ranges() {
        // ARRANGE
        let open = Range::from_start(1024);
        let closed = Range {
            start: 1024,
            end: Some(2047),
        };

        // ACT
        let (open_value, closed_value) = (open.header_value(), closed.header_value());

        // ASSERT
        assert_eq!(open_value, "bytes=1024-");
        assert_eq!(closed_value, "bytes=1024-2047");
    }

    #[test]
    fn is_partial_recognizes_only_partial_content() {
        // ARRANGE
        let partial = Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .body(())
            .expect("build response");
        let full = Response::builder()
            .status(StatusCode::OK)
            .body(())
            .expect("build response");

        // ACT / ASSERT
        assert!(is_partial(&partial));
        assert!(!is_partial(&full));
    }
}
