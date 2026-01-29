//! TLS connector for insecure connections (TOFU model).

use std::sync::Arc;

use anyhow::{Context, Result};
use hyper::Uri;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// TLS connector that skips certificate verification (TOFU model).
#[derive(Clone)]
pub struct InsecureTlsConnector {
    tls_connector: tokio_rustls::TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
    server_addr: std::net::SocketAddr,
}

impl InsecureTlsConnector {
    pub fn new(server: &str) -> Result<Self> {
        let server_addr: std::net::SocketAddr = server
            .parse()
            .with_context(|| format!("Invalid server address: {}", server))?;

        let mut tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
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

impl tower::Service<Uri> for InsecureTlsConnector {
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

/// Server certificate verifier that accepts any certificate (TOFU model).
#[derive(Debug)]
struct InsecureServerCertVerifier;

impl ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
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
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}
