//! HTTP/2 server connection handling

use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls_pki_types::CertificateDer;
use std::net::SocketAddr;
use tokio_rustls::server::TlsStream;

use crate::handler;
use crate::tls;

/// Serves a TLS-wrapped connection.
pub async fn serve_tls_connection(
    tls_stream: TlsStream<tokio::net::TcpStream>,
    peer_addr: SocketAddr,
    client_cert: Option<CertificateDer<'static>>,
    maintenance_mode: bool,
) {
    let io = TokioIo::new(tls_stream);
    let client_fingerprint = client_cert.map(|cert| tls::extract_fingerprint(&cert));

    let service = service_fn(move |req| {
        let fingerprint = client_fingerprint.clone();
        async move { handler::handle_request(req, fingerprint, maintenance_mode).await }
    });

    let conn = http2::Builder::new(TokioExecutor::new()).serve_connection(io, service);

    if let Err(e) = conn.await {
        kmsg::warn!("Connection error from {}: {}", peer_addr, e);
    }
}
