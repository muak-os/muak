//! TLS connectors for TOFU (Trust On First Use) enrollment model.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use hyper::Uri;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Shared state for capturing the server certificate fingerprint during TOFU.
#[derive(Clone, Debug, Default)]
pub struct TofuState {
    inner: Arc<Mutex<Option<String>>>,
}

impl TofuState {
    /// Returns the captured server fingerprint, if one has been recorded.
    pub fn fingerprint(&self) -> Option<String> {
        self.inner.lock().unwrap().clone()
    }
}

/// TLS connector that captures the server certificate fingerprint on first use.
#[derive(Clone)]
pub struct TofuTlsConnector {
    tls_connector: tokio_rustls::TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
    server_addr: std::net::SocketAddr,
}

impl TofuTlsConnector {
    pub fn new(server: &str, state: TofuState) -> Result<Self> {
        let server_addr: std::net::SocketAddr = server
            .parse()
            .with_context(|| format!("Invalid server address: {}", server))?;

        let verifier = TofuServerCertVerifier {
            state,
            pinned_fingerprint: None,
        };

        let mut tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();

        tls_config.alpn_protocols = vec![b"h2".to_vec()];

        let tls_connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));

        let server_name = rustls::pki_types::ServerName::IpAddress(
            rustls::pki_types::IpAddr::from(server_addr.ip()),
        );

        Ok(Self {
            tls_connector,
            server_name,
            server_addr,
        })
    }
}

impl tonic::codegen::Service<Uri> for TofuTlsConnector {
    type Response = hyper_util::rt::TokioIo<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let tls_connector = self.tls_connector.clone();
        let server_name = self.server_name.clone();
        let server_addr = self.server_addr;

        Box::pin(async move {
            let tcp_stream = tokio::net::TcpStream::connect(server_addr).await?;

            let tls_stream = tls_connector
                .connect(server_name, tcp_stream)
                .await
                .map_err(std::io::Error::other)?;

            Ok(hyper_util::rt::TokioIo::new(tls_stream))
        })
    }
}

/// TLS connector that only accepts a server with a specific certificate fingerprint.
#[derive(Clone)]
pub struct PinnedTlsConnector {
    tls_connector: tokio_rustls::TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
    server_addr: std::net::SocketAddr,
}

impl PinnedTlsConnector {
    pub fn new(server: &str, fingerprint: &str) -> Result<Self> {
        let server_addr: std::net::SocketAddr = server
            .parse()
            .with_context(|| format!("Invalid server address: {}", server))?;

        let verifier = TofuServerCertVerifier {
            state: TofuState::default(),
            pinned_fingerprint: Some(fingerprint.to_string()),
        };

        let mut tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();

        tls_config.alpn_protocols = vec![b"h2".to_vec()];

        let tls_connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));

        let server_name = rustls::pki_types::ServerName::IpAddress(
            rustls::pki_types::IpAddr::from(server_addr.ip()),
        );

        Ok(Self {
            tls_connector,
            server_name,
            server_addr,
        })
    }
}

impl tonic::codegen::Service<Uri> for PinnedTlsConnector {
    type Response = hyper_util::rt::TokioIo<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let tls_connector = self.tls_connector.clone();
        let server_name = self.server_name.clone();
        let server_addr = self.server_addr;

        Box::pin(async move {
            let tcp_stream = tokio::net::TcpStream::connect(server_addr).await?;

            let tls_stream = tls_connector
                .connect(server_name, tcp_stream)
                .await
                .map_err(std::io::Error::other)?;

            Ok(hyper_util::rt::TokioIo::new(tls_stream))
        })
    }
}

/// Server certificate verifier implementing TOFU semantics.
#[derive(Debug)]
struct TofuServerCertVerifier {
    state: TofuState,
    pinned_fingerprint: Option<String>,
}

impl ServerCertVerifier for TofuServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint = pki::util::to_hex(
            ring::digest::digest(&ring::digest::SHA256, end_entity.as_ref()).as_ref(),
        );

        if let Some(pinned) = &self.pinned_fingerprint
            && fingerprint != *pinned {
                return Err(rustls::Error::General(format!(
                    "Server certificate fingerprint mismatch: expected {}, got {}",
                    &pinned[..16],
                    &fingerprint[..16],
                )));
            }

        *self.state.inner.lock().unwrap() = Some(fingerprint);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}
