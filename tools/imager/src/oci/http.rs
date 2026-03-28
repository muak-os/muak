//! Shared HTTP/HTTPS client and low-level request helpers for OCI registry communication.

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::{ClientConfig, RootCertStore};

use crate::error::{ImagerError, Result};
use crate::oci::USER_AGENT;

/// HTTPS connector backed by rustls that also supports plain HTTP.
type HttpsConnector = hyper_rustls::HttpsConnector<HttpConnector>;

/// Cloneable HTTP/HTTPS client for all registries.
pub(crate) type HttpClient = Client<HttpsConnector, Full<Bytes>>;

/// Build a reusable client supporting both HTTPS and plain HTTP.
pub(crate) fn build_client() -> Result<HttpClient> {
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

    Ok(Client::builder(TokioExecutor::new()).build(connector))
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
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {}", t));
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

    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {}", t));
    }

    send(client, builder.body(Full::new(body)), url).await
}

/// Dispatch a pre-built request and validate the response status.
async fn send(
    client: &HttpClient,
    req: std::result::Result<Request<Full<Bytes>>, hyper::http::Error>,
    url: &str,
) -> Result<Response<Incoming>> {
    let req =
        req.map_err(|e| ImagerError::NetworkError(format!("Failed to build request: {}", e)))?;
    let resp = client
        .request(req)
        .await
        .map_err(|e| ImagerError::NetworkError(format!("HTTP request failed: {}", e)))?;
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(ImagerError::DownloadError(format!(
            "HTTP {} for URL: {}",
            resp.status(),
            url
        )))
    }
}

/// Fully collect an HTTP response body into [`Bytes`].
pub(crate) async fn collect_body(resp: Response<Incoming>) -> Result<Bytes> {
    resp.into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .map_err(|e| ImagerError::NetworkError(format!("Failed to read response body: {}", e)))
}
