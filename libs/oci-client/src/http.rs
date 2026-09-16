//! Shared HTTP/HTTPS client and low-level request helpers for OCI registry communication.

use core::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::http::request::Builder;
use hyper::{Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use oci::digest::Verifier;
use tokio::time::timeout;

use crate::error::{ClientError, Result};
use crate::redirect;

const HTTP_TIMEOUT: Duration = Duration::from_mins(1);

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.3";

#[cfg(feature = "https")]
type Connector = hyper_rustls::HttpsConnector<HttpConnector>;

#[cfg(not(feature = "https"))]
type Connector = HttpConnector;

/// Cloneable HTTP client for all registries.
pub type Transport = Client<Connector, Full<Bytes>>;

/// Build a reusable client supporting both HTTPS and plain HTTP.
#[cfg(feature = "https")]
#[must_use]
pub fn build_client() -> Transport {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = rustls::ClientConfig::builder()
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

/// Build a reusable plain-HTTP client.
#[cfg(not(feature = "https"))]
#[must_use]
pub fn build_client() -> Transport {
    Client::builder(TokioExecutor::new()).build(HttpConnector::new())
}

/// Execute an authorized GET, returning the response on 2xx.
///
/// # Errors
///
/// Returns an error when the request fails or the registry answers non-2xx.
pub async fn get(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Response<Incoming>> {
    let response = redirect::follow(client, url, authorization, accept_headers).await?;

    ensure_success(url, response)
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
) -> Result<Response<Incoming>> {
    let request = get_request(url, authorization, accept_headers)?;

    send(client, url, request).await
}

/// Execute a HEAD and return the response whatever its status.
///
/// # Errors
///
/// Returns an error when the request cannot be built or sent.
pub async fn head_any_status(
    client: &Transport,
    url: &str,
    authorization: Option<&str>,
) -> Result<Response<Incoming>> {
    let request = head_request(url, authorization)?;

    send(client, url, request).await
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
    let request = post_request(url, authorization)?;

    send(client, url, request).await
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
    let request = put_request(url, authorization, content_type, body)?;
    let response = send(client, url, request).await?;

    ensure_success(url, response)
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

/// Stream an HTTP response body into memory while computing a digest.
///
/// # Errors
///
/// Returns an error when reading the body times out or fails.
pub async fn stream_body_to_vec(
    resp: Response<Incoming>,
    digest: &mut Verifier,
) -> Result<Vec<u8>> {
    let mut body = resp.into_body();
    let mut bytes = Vec::new();

    while let Some(frame) = timeout(HTTP_TIMEOUT, body.frame()).await.map_err(|error| {
        ClientError::Network(format!(
            "HTTP response body timed out after {HTTP_TIMEOUT:?}: {error}"
        ))
    })? {
        let frame = frame.map_err(|error| {
            ClientError::Network(format!("Failed to read response body: {error}"))
        })?;

        if let Some(data) = frame.data_ref() {
            bytes.extend_from_slice(data);
            digest.update(data);
        }
    }

    Ok(bytes)
}

fn get_request(
    url: &str,
    authorization: Option<&str>,
    accept_headers: &[&str],
) -> Result<Request<Full<Bytes>>> {
    let mut builder = base_request(Method::GET, url);
    for accept in accept_headers {
        builder = builder.header("Accept", *accept);
    }

    finish_request(builder, authorization, Full::new(Bytes::new()))
}

fn head_request(url: &str, authorization: Option<&str>) -> Result<Request<Full<Bytes>>> {
    finish_request(
        base_request(Method::HEAD, url),
        authorization,
        Full::new(Bytes::new()),
    )
}

fn post_request(url: &str, authorization: Option<&str>) -> Result<Request<Full<Bytes>>> {
    finish_request(
        base_request(Method::POST, url),
        authorization,
        Full::new(Bytes::new()),
    )
}

fn put_request(
    url: &str,
    authorization: Option<&str>,
    content_type: &str,
    body: Bytes,
) -> Result<Request<Full<Bytes>>> {
    let builder = base_request(Method::PUT, url).header("Content-Type", content_type);

    finish_request(builder, authorization, Full::new(body))
}

fn base_request(method: Method, url: &str) -> Builder {
    Request::builder()
        .method(method)
        .uri(url)
        .header("User-Agent", USER_AGENT)
}

fn finish_request(
    builder: Builder,
    authorization: Option<&str>,
    body: Full<Bytes>,
) -> Result<Request<Full<Bytes>>> {
    let builder = match authorization {
        Some(value) => builder.header("Authorization", value),
        None => builder,
    };

    builder
        .body(body)
        .map_err(|error| ClientError::Network(format!("Failed to build request: {error}")))
}

async fn send(
    client: &Transport,
    url: &str,
    request: Request<Full<Bytes>>,
) -> Result<Response<Incoming>> {
    timeout(HTTP_TIMEOUT, client.request(request))
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
        Err(ClientError::Download(format!(
            "HTTP {} for URL: {}",
            response.status(),
            url
        )))
    }
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
        assert!(matches!(error, ClientError::Network(_)));
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
        assert!(matches!(error, ClientError::Network(_)));
    }

    #[tokio::test]
    async fn head_rejects_invalid_url_before_request() {
        // ARRANGE
        let client = build_client();

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
        let client = build_client();

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
        assert!(matches!(error, ClientError::Network(_)));
    }
}
