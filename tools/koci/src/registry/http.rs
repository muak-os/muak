//! Shared HTTP/HTTPS client and low-level request helpers for OCI registry communication.

use core::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::{ClientConfig, RootCertStore};
use tokio::time::timeout;

use crate::digest::StreamingDigest;
use crate::error::{KociError, Result};
use crate::registry::USER_AGENT;

const HTTP_TIMEOUT: Duration = Duration::from_mins(1);

/// HTTPS connector backed by rustls that also supports plain HTTP.
type HttpsConnector = hyper_rustls::HttpsConnector<HttpConnector>;

/// Cloneable HTTP/HTTPS client for all registries.
pub(crate) type HttpClient = Client<HttpsConnector, Full<Bytes>>;

/// Build a reusable client supporting both HTTPS and plain HTTP.
pub(crate) fn build_client() -> HttpClient {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    Client::builder(TokioExecutor::new()).build(connector)
}

/// Execute an authorized GET, returning the response on 2xx.
pub(crate) async fn get(
    client: &HttpClient,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    let response = get_any_status(client, url, authorization, accept_headers).await?;
    ensure_success(url, response)
}

/// Execute a GET and return the response whatever its status.
pub(crate) async fn get_any_status(
    client: &HttpClient,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    let request = get_request(url, authorization, accept_headers)?;

    send(client, url, request).await
}

/// Execute an authorized PUT with a raw body, returning the response on 2xx.
pub(crate) async fn put(
    client: &HttpClient,
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Response<Incoming>> {
    let request = put_request(url, authorization, content_type, body)?;
    let response = send(client, url, request).await?;

    ensure_success(url, response)
}

/// Build a GET request with optional authorization and Accept headers.
fn get_request(
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Request<Full<Bytes>>> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("User-Agent", USER_AGENT);

    for accept in accept_headers {
        builder = builder.header("Accept", *accept);
    }
    if let Some(value) = authorization {
        builder = builder.header("Authorization", value);
    }

    builder
        .body(Full::new(Bytes::new()))
        .map_err(|error| KociError::NetworkError(format!("Failed to build request: {error}")))
}

/// Build a PUT request with optional authorization and a raw body.
fn put_request(
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Request<Full<Bytes>>> {
    let mut builder = Request::builder()
        .method(Method::PUT)
        .uri(url)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", content_type);

    if let Some(value) = authorization {
        builder = builder.header("Authorization", value);
    }

    builder
        .body(Full::new(body))
        .map_err(|error| KociError::NetworkError(format!("Failed to build request: {error}")))
}

/// Dispatch a pre-built request, ignoring the response status.
async fn send(
    client: &HttpClient,
    url: &str,
    request: Request<Full<Bytes>>,
) -> Result<Response<Incoming>> {
    timeout(HTTP_TIMEOUT, client.request(request))
        .await
        .map_err(|error| {
            KociError::NetworkError(format!(
                "HTTP request timed out after {HTTP_TIMEOUT:?} for URL: {url}: {error}"
            ))
        })?
        .map_err(|error| KociError::NetworkError(format!("HTTP request failed: {error}")))
}

/// Map a non-2xx response to a download error.
fn ensure_success(url: &str, response: Response<Incoming>) -> Result<Response<Incoming>> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(KociError::DownloadError(format!(
            "HTTP {} for URL: {}",
            response.status(),
            url
        )))
    }
}

/// Fully collect an HTTP response body into [`Bytes`].
pub(crate) async fn collect_body(resp: Response<Incoming>) -> Result<Bytes> {
    timeout(HTTP_TIMEOUT, resp.into_body().collect())
        .await
        .map_err(|error| {
            KociError::NetworkError(format!(
                "HTTP response body timed out after {HTTP_TIMEOUT:?}: {error}"
            ))
        })?
        .map(http_body_util::Collected::to_bytes)
        .map_err(|error| KociError::NetworkError(format!("Failed to read response body: {error}")))
}

/// Stream an HTTP response body into memory while computing a digest.
pub(crate) async fn stream_body_to_vec(
    resp: Response<Incoming>,
    digest: &mut StreamingDigest,
) -> Result<Vec<u8>> {
    let mut body = resp.into_body();
    let mut bytes = Vec::new();

    while let Some(frame) = timeout(HTTP_TIMEOUT, body.frame()).await.map_err(|error| {
        KociError::NetworkError(format!(
            "HTTP response body timed out after {HTTP_TIMEOUT:?}: {error}"
        ))
    })? {
        let frame = frame.map_err(|error| {
            KociError::NetworkError(format!("Failed to read response body: {error}"))
        })?;

        if let Some(data) = frame.data_ref() {
            bytes.extend_from_slice(data);
            digest.update(data);
        }
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = build_client();

        // ACT
        let error = get(&client, "http://127.0.0.1:5000/has space", None, &[])
            .await
            .expect_err("request should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[tokio::test]
    async fn put_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = build_client();

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
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[tokio::test]
    async fn get_reports_connection_failures() {
        // ARRANGE
        let client = build_client();

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
        assert!(matches!(error, KociError::NetworkError(_)));
    }
}
