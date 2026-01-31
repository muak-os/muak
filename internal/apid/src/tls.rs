//! TLS configuration for mTLS authentication

use std::sync::Arc;

use anyhow::{Context, Result};
use der::Encode;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

use crate::config;

/// Loads TLS config from disk (installed system).
pub fn load_tls_config() -> Result<TlsAcceptor> {
    let ca_certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(config::CA_CERT_PATH)
        .context("Failed to read CA certificate")?
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

    let server_certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(config::SERVER_CERT_PATH)
        .context("Failed to read server certificate")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse server certificate")?;

    let server_key = PrivateKeyDer::from_pem_file(config::SERVER_KEY_PATH)
        .context("Failed to read/parse server private key")?;

    let mut server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .context("Failed to build server config")?;

    server_config.alpn_protocols = vec![b"h2".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// Generates ephemeral TLS config in memory (used in maintenance mode).
pub fn generate_ephemeral_tls_config() -> Result<TlsAcceptor> {
    let (ca_signer, ca_cert) =
        pki::generate_ca_certificate("Muak Ephemeral CA").context("Failed to generate CA")?;

    let (server_signer, server_cert) =
        pki::generate_server_certificate("muak-server", &ca_signer, &ca_cert)
            .context("Failed to generate server certificate")?;

    let ca_cert_der = ca_cert
        .to_der()
        .context("Failed to encode CA certificate")?;
    let server_cert_der = server_cert
        .to_der()
        .context("Failed to encode server certificate")?;
    let server_key_der = server_signer.pkcs8_der().to_vec();

    let mut root_store = RootCertStore::empty();
    root_store
        .add(CertificateDer::from(ca_cert_der))
        .context("Failed to add CA cert to root store")?;

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .allow_unauthenticated()
        .build()
        .context("Failed to build client verifier")?;

    let server_certs = vec![CertificateDer::from(server_cert_der)];
    let server_key = PrivateKeyDer::try_from(server_key_der)
        .map_err(|e| anyhow::anyhow!("Failed to parse server key: {}", e))?;

    let mut server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .context("Failed to build server config")?;

    server_config.alpn_protocols = vec![b"h2".to_vec()];

    kmsg::info!("Ephemeral TLS certificates generated for maintenance mode");

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// Extracts SHA256 fingerprint from a DER-encoded certificate.
pub fn extract_fingerprint(cert_der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert_der);
    pki::util::to_hex(digest.as_ref())
}
