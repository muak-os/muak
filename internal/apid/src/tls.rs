//! TLS configuration for mTLS authentication

use crate::config;
use anyhow::{Context, Result};
use rustls::pki_types::CertificateDer;
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use std::fs;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

/// Loads TLS config that allows optional client certs (for maintenance mode endpoints).
/// Returns `None` if TLS certificates are not present.
pub fn load_tls_config() -> Result<Option<TlsAcceptor>> {
    if !std::path::Path::new(config::CA_CERT_PATH).exists()
        || !std::path::Path::new(config::SERVER_CERT_PATH).exists()
        || !std::path::Path::new(config::SERVER_KEY_PATH).exists()
    {
        return Ok(None);
    }

    let ca_pem = fs::read(config::CA_CERT_PATH).context("Failed to read CA certificate")?;
    let ca_certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut ca_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse CA certificate")?;

    let mut root_store = RootCertStore::empty();
    for cert in ca_certs {
        root_store
            .add(cert)
            .context("Failed to add CA cert to root store")?;
    }

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .allow_unauthenticated()
        .build()
        .context("Failed to build client verifier")?;

    let server_cert_pem =
        fs::read(config::SERVER_CERT_PATH).context("Failed to read server certificate")?;
    let server_certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut server_cert_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse server certificate")?;

    let server_key_pem =
        fs::read(config::SERVER_KEY_PATH).context("Failed to read server private key")?;
    let server_key = rustls_pemfile::private_key(&mut server_key_pem.as_slice())
        .context("Failed to parse server private key")?
        .ok_or_else(|| anyhow::anyhow!("No private key found in server key file"))?;

    let mut server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .context("Failed to build server config")?;

    server_config.alpn_protocols = vec![b"h2".to_vec()];

    Ok(Some(TlsAcceptor::from(Arc::new(server_config))))
}

/// Extracts SHA256 fingerprint from a DER-encoded certificate
pub fn extract_fingerprint(cert_der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert_der);
    hex::encode(digest.as_ref())
}
