//! Shared HTTP transports and client construction.

use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use http_body_util::Full;
use hyper::body::Bytes;
#[cfg(feature = "https")]
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
#[cfg(feature = "https")]
type Connector = hyper_rustls::HttpsConnector<HttpConnector>;

#[cfg(not(feature = "https"))]
type Connector = HttpConnector;

/// Cloneable HTTP client for all registries.
pub type Transport = Client<Connector, Full<Bytes>>;

/// Deadline for establishing a TCP connection to a registry.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared transports per registry host, so connection pools and TLS sessions
/// are reused across pulls.
static TRANSPORTS: OnceLock<Mutex<HashMap<String, Transport>>> = OnceLock::new();

/// Build a reusable client supporting both HTTPS and plain HTTP.
#[cfg(feature = "https")]
#[must_use]
pub fn build_client() -> Transport {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let mut http_connector = HttpConnector::new();
    http_connector.set_connect_timeout(Some(CONNECT_TIMEOUT));
    http_connector.enforce_http(false);

    let connector = HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http_connector);

    Client::builder(TokioExecutor::new()).build(connector)
}

/// Build a reusable plain-HTTP client.
#[cfg(not(feature = "https"))]
#[must_use]
pub fn build_client() -> Transport {
    let mut http_connector = HttpConnector::new();
    http_connector.set_connect_timeout(Some(CONNECT_TIMEOUT));

    Client::builder(TokioExecutor::new()).build(http_connector)
}

/// Return the process-shared transport for a registry host, creating it once.
#[must_use]
pub fn for_host(host: &str) -> Transport {
    let transports = TRANSPORTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut transports = transports.lock().unwrap_or_else(PoisonError::into_inner);

    transports
        .entry(host.to_owned())
        .or_insert_with(build_client)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_transport_count() -> usize {
        TRANSPORTS.get().map_or(0, |transports| {
            transports
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
        })
    }

    #[test]
    fn for_host_reuses_one_transport_per_host() {
        // ARRANGE
        let first = for_host("127.0.0.1:5001");
        let second = for_host("127.0.0.1:5001");

        // ACT
        let other = for_host("127.0.0.1:5002");

        // ASSERT
        assert_eq!(shared_transport_count(), 2);
        drop((first, second, other));
    }
}
