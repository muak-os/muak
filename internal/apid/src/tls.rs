//! TLS configuration for mTLS authentication

use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;
use x509_cert::der::Encode;

use crate::config;

/// Loads TLS config from default disk paths.
pub fn load_tls_config() -> Result<TlsAcceptor> {
    load_tls_config_with_paths(
        config::CA_CERT_PATH,
        config::SERVER_CERT_PATH,
        config::SERVER_KEY_PATH,
    )
}

/// Loads TLS config from disk with custom paths (used for testing).
pub fn load_tls_config_with_paths(
    ca_cert_path: &str,
    server_cert_path: &str,
    server_key_path: &str,
) -> Result<TlsAcceptor> {
    let ca_certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(ca_cert_path)
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

    let server_certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(server_cert_path)
        .context("Failed to read server certificate")?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse server certificate")?;

    let server_key = PrivateKeyDer::from_pem_file(server_key_path)
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn test_extract_fingerprint_returns_64_char_hex() {
        let test_data = b"test certificate data";
        let fingerprint = extract_fingerprint(test_data);

        assert_eq!(
            fingerprint.len(),
            64,
            "SHA256 fingerprint should be 64 hex characters"
        );
    }

    #[test]
    fn test_extract_fingerprint_is_lowercase_hex() {
        let test_data = b"test certificate data";
        let fingerprint = extract_fingerprint(test_data);

        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Fingerprint should be lowercase hex: {}",
            fingerprint
        );
    }

    #[test]
    fn test_extract_fingerprint_deterministic() {
        let test_data = b"same input data";

        let fp1 = extract_fingerprint(test_data);
        let fp2 = extract_fingerprint(test_data);

        assert_eq!(fp1, fp2, "Same input should produce same fingerprint");
    }

    #[test]
    fn test_extract_fingerprint_different_inputs() {
        let fp1 = extract_fingerprint(b"first certificate");
        let fp2 = extract_fingerprint(b"second certificate");

        assert_ne!(
            fp1, fp2,
            "Different inputs should produce different fingerprints"
        );
    }

    #[test]
    fn test_extract_fingerprint_empty_input() {
        let fingerprint = extract_fingerprint(&[]);

        assert_eq!(fingerprint.len(), 64);
        assert_eq!(
            fingerprint,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_extract_fingerprint_with_real_cert() {
        let (_, ca_cert) =
            pki::generate_ca_certificate("Test CA").expect("Failed to generate test CA");
        let cert_der =
            x509_cert::der::Encode::to_der(&ca_cert).expect("Failed to encode certificate to DER");

        let fingerprint = extract_fingerprint(&cert_der);

        assert_eq!(
            fingerprint.len(),
            64,
            "Real cert fingerprint should be 64 chars"
        );
        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Fingerprint should be lowercase hex"
        );
    }

    #[test]
    fn test_extract_fingerprint_matches_pki_compute() {
        let (_, cert) =
            pki::generate_ca_certificate("Test CA").expect("Failed to generate test CA");
        let cert_der =
            x509_cert::der::Encode::to_der(&cert).expect("Failed to encode certificate to DER");

        let our_fingerprint = extract_fingerprint(&cert_der);
        let pki_fingerprint =
            pki::compute_cert_fingerprint(&cert).expect("Failed to compute pki fingerprint");

        assert_eq!(
            our_fingerprint, pki_fingerprint,
            "Our fingerprint should match pki crate's computation"
        );
    }

    #[test]
    fn test_generate_ephemeral_tls_config() {
        let result = generate_ephemeral_tls_config();

        assert!(
            result.is_ok(),
            "Should be able to generate ephemeral TLS config: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_load_tls_config_with_valid_paths() {
        let (ca_signer, ca_cert) =
            pki::generate_ca_certificate("Test CA").expect("Failed to generate CA");
        let (server_signer, server_cert) =
            pki::generate_server_certificate("test-server", &ca_signer, &ca_cert)
                .expect("Failed to generate server cert");

        let ca_cert_der = ca_cert.to_der().expect("Failed to encode CA cert");
        let server_cert_der = server_cert.to_der().expect("Failed to encode server cert");
        let server_key_der = server_signer.pkcs8_der();

        let ca_doc =
            x509_cert::der::Document::try_from(ca_cert_der).expect("Failed to create document");
        let ca_cert_pem = ca_doc
            .to_pem("CERTIFICATE", x509_cert::der::pem::LineEnding::LF)
            .expect("Failed to convert CA to PEM");

        let server_doc =
            x509_cert::der::Document::try_from(server_cert_der).expect("Failed to create document");
        let server_cert_pem = server_doc
            .to_pem("CERTIFICATE", x509_cert::der::pem::LineEnding::LF)
            .expect("Failed to convert server cert to PEM");

        let server_key_pem =
            pki::util::pkcs8_to_pem(server_key_der).expect("Failed to convert server key to PEM");

        let mut ca_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let mut server_cert_file =
            tempfile::NamedTempFile::new().expect("Failed to create temp file");
        let mut server_key_file =
            tempfile::NamedTempFile::new().expect("Failed to create temp file");

        ca_file
            .write_all(ca_cert_pem.as_bytes())
            .expect("Failed to write CA cert");
        server_cert_file
            .write_all(server_cert_pem.as_bytes())
            .expect("Failed to write server cert");
        server_key_file
            .write_all(server_key_pem.as_bytes())
            .expect("Failed to write server key");

        let result = load_tls_config_with_paths(
            ca_file.path().to_str().unwrap(),
            server_cert_file.path().to_str().unwrap(),
            server_key_file.path().to_str().unwrap(),
        );

        assert!(
            result.is_ok(),
            "Should load TLS config from valid paths: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_load_tls_config_with_missing_ca() {
        let result = load_tls_config_with_paths(
            "/nonexistent/ca.crt",
            "/nonexistent/server.crt",
            "/nonexistent/server.key",
        );

        assert!(result.is_err(), "Should fail with missing CA cert");
        match result {
            Err(e) => {
                assert!(
                    e.to_string().contains("CA certificate"),
                    "Error should mention CA certificate, got: {}",
                    e
                );
            }
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }
}
