//! Shared HTTP/HTTPS client and low-level request helpers for OCI registry communication.

use core::result::Result as CoreResult;
use core::time::Duration;
use std::fs::File;
use std::io::Write as _;

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::http::Error as HttpError;
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

/// Execute an authenticated GET, returning the response on 2xx.
pub(crate) async fn get(
    client: &HttpClient,
    url: &str,
    token: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("User-Agent", USER_AGENT);

    for accept in accept_headers {
        builder = builder.header("Accept", *accept);
    }
    if let Some(token_value) = token {
        builder = builder.header("Authorization", format!("Bearer {token_value}"));
    }

    send(client, builder.body(Full::new(Bytes::new())), url).await
}

/// Execute an authenticated PUT with a raw body, returning the response on 2xx.
pub(crate) async fn put(
    client: &HttpClient,
    url: &str,
    token: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Response<Incoming>> {
    let mut builder = Request::builder()
        .method(Method::PUT)
        .uri(url)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", content_type);

    if let Some(token_value) = token {
        builder = builder.header("Authorization", format!("Bearer {token_value}"));
    }

    send(client, builder.body(Full::new(body)), url).await
}

/// Dispatch a pre-built request and validate the response status.
async fn send(
    client: &HttpClient,
    req: CoreResult<Request<Full<Bytes>>, HttpError>,
    url: &str,
) -> Result<Response<Incoming>> {
    let req =
        req.map_err(|error| KociError::NetworkError(format!("Failed to build request: {error}")))?;
    let resp = timeout(HTTP_TIMEOUT, client.request(req))
        .await
        .map_err(|error| {
            KociError::NetworkError(format!(
                "HTTP request timed out after {HTTP_TIMEOUT:?} for URL: {url}: {error}"
            ))
        })?
        .map_err(|error| KociError::NetworkError(format!("HTTP request failed: {error}")))?;
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(KociError::DownloadError(format!(
            "HTTP {} for URL: {}",
            resp.status(),
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

/// Stream an HTTP response body to a file while computing a digest.
pub(crate) async fn stream_body_to_file(
    resp: Response<Incoming>,
    file: &mut File,
    digest: &mut StreamingDigest,
) -> Result<()> {
    let mut body = resp.into_body();

    while let Some(frame) = timeout(HTTP_TIMEOUT, body.frame()).await.map_err(|error| {
        KociError::NetworkError(format!(
            "HTTP response body timed out after {HTTP_TIMEOUT:?}: {error}"
        ))
    })? {
        let frame = frame.map_err(|error| {
            KociError::NetworkError(format!("Failed to read response body: {error}"))
        })?;

        if let Some(data) = frame.data_ref() {
            file.write_all(data)?;
            digest.update(data);
        }
    }

    Ok(())
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
