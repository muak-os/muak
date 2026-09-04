//! TLS configuration for mTLS authentication.

extern crate alloc;

use alloc::sync::Arc;

use anyhow::{Context as _, Result};
use pki::cert;
use pki::hex;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use sha2::{Digest as _, Sha256};
use tokio_rustls::TlsAcceptor;
use x509_cert::der::Encode as _;

use crate::constants;

/// Loads TLS config from default disk paths.
///
/// # Errors
///
/// Returns an error if the certificate, key, or CA files cannot be read.
pub fn load_tls_config() -> Result<TlsAcceptor> {
    load_tls_config_with_paths(
        constants::CA_CERT_PATH,
        constants::SERVER_CERT_PATH,
        constants::SERVER_KEY_PATH,
    )
}

/// Loads TLS config from disk with custom paths (used for testing).
///
/// # Errors
///
/// Returns an error if the certificate, key, or CA files cannot be read.
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
    for cert_der in ca_certs {
        root_store
            .add(cert_der)
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
///
/// # Errors
///
/// Returns an error if the ephemeral CA or server certificate cannot be
/// generated.
pub fn generate_ephemeral_tls_config() -> Result<TlsAcceptor> {
    let (ca_signer, ca_cert) =
        cert::generate_ca("Muak Ephemeral CA").context("Failed to generate CA")?;

    let (server_signer, server_cert) = cert::generate_server("muak-server", &ca_signer, &ca_cert)
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
        .map_err(|e| anyhow::anyhow!("Failed to parse server key: {e}"))?;

    let mut server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .context("Failed to build server config")?;

    server_config.alpn_protocols = vec![b"h2".to_vec()];

    println!("Ephemeral TLS certificates generated for maintenance mode");

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// Extracts SHA256 fingerprint from a DER-encoded certificate.
#[must_use]
pub fn extract_fingerprint(cert_der: &[u8]) -> String {
    hex::encode_lower(Sha256::digest(cert_der).as_ref())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use pki::key::Signer;
    use pki::pem::encode_pkcs8;
    use x509_cert::Certificate;
    use x509_cert::der::Document;
    use x509_cert::der::pem::LineEnding;

    use super::*;

    /// Generates a test CA certificate and returns `(signer, cert, cert_der)`.
    fn make_test_ca() -> (Signer, Certificate, Vec<u8>) {
        let (signer, cert) = cert::generate_ca("Test CA").expect("Failed to generate test CA");
        let cert_der = cert.to_der().expect("Failed to encode certificate to DER");
        (signer, cert, cert_der)
    }

    #[test]
    fn extract_fingerprint_returns_64_char_hex() {
        // ARRANGE
        let test_data = b"test certificate data";

        // ACT
        let fingerprint = extract_fingerprint(test_data);

        // ASSERT
        assert_eq!(
            fingerprint.len(),
            64,
            "SHA256 fingerprint should be 64 hex characters"
        );
    }

    #[test]
    fn extract_fingerprint_is_lowercase_hex() {
        // ARRANGE
        let test_data = b"test certificate data";

        // ACT
        let fingerprint = extract_fingerprint(test_data);

        // ASSERT
        assert!(
            fingerprint
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "Fingerprint should be lowercase hex: {fingerprint}"
        );
    }

    #[test]
    fn extract_fingerprint_deterministic() {
        // ARRANGE
        let test_data = b"same input data";

        // ACT
        let fp1 = extract_fingerprint(test_data);
        let fp2 = extract_fingerprint(test_data);

        // ASSERT
        assert_eq!(fp1, fp2, "Same input should produce same fingerprint");
    }

    #[test]
    fn extract_fingerprint_different_inputs() {
        // ARRANGE
        let fp1 = extract_fingerprint(b"first certificate");
        let fp2 = extract_fingerprint(b"second certificate");

        // ASSERT
        assert_ne!(
            fp1, fp2,
            "Different inputs should produce different fingerprints"
        );
    }

    #[test]
    fn extract_fingerprint_empty_input() {
        // ACT
        let fingerprint = extract_fingerprint(&[]);

        // ASSERT
        assert_eq!(fingerprint.len(), 64);
        assert_eq!(
            fingerprint,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn extract_fingerprint_with_real_cert() {
        // ARRANGE
        let (_, _, cert_der) = make_test_ca();

        // ACT
        let fingerprint = extract_fingerprint(&cert_der);

        // ASSERT
        assert_eq!(
            fingerprint.len(),
            64,
            "Real cert fingerprint should be 64 chars"
        );
        assert!(
            fingerprint
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "Fingerprint should be lowercase hex"
        );
    }

    #[test]
    fn extract_fingerprint_matches_pki_compute() {
        // ARRANGE
        let (_, cert, cert_der) = make_test_ca();

        // ACT
        let our_fingerprint = extract_fingerprint(&cert_der);
        let pki_fingerprint =
            cert::compute_fingerprint(&cert).expect("Failed to compute pki fingerprint");

        // ASSERT
        assert_eq!(
            our_fingerprint, pki_fingerprint,
            "Our fingerprint should match pki crate's computation"
        );
    }

    #[test]
    fn generate_ephemeral_tls_config_succeeds() {
        // ACT
        let result = generate_ephemeral_tls_config();

        // ASSERT
        assert!(
            result.is_ok(),
            "Should be able to generate ephemeral TLS config: {:?}",
            result.err()
        );
    }

    #[test]
    fn load_tls_config_with_valid_paths() {
        // ARRANGE
        let (ca_signer, ca_cert, _) = make_test_ca();
        let (server_signer, server_cert) =
            cert::generate_server("test-server", &ca_signer, &ca_cert)
                .expect("Failed to generate server cert");

        let ca_cert_der = ca_cert.to_der().expect("Failed to encode CA cert");
        let server_cert_der = server_cert.to_der().expect("Failed to encode server cert");
        let server_key_der = server_signer.pkcs8_der();

        let ca_doc = Document::try_from(ca_cert_der).expect("Failed to create document");
        let ca_cert_pem = ca_doc
            .to_pem("CERTIFICATE", LineEnding::LF)
            .expect("Failed to convert CA to PEM");

        let server_doc = Document::try_from(server_cert_der).expect("Failed to create document");
        let server_cert_pem = server_doc
            .to_pem("CERTIFICATE", LineEnding::LF)
            .expect("Failed to convert server cert to PEM");

        let server_key_pem =
            encode_pkcs8(server_key_der).expect("Failed to convert server key to PEM");

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

        // ACT
        let result = load_tls_config_with_paths(
            ca_file.path().to_str().unwrap(),
            server_cert_file.path().to_str().unwrap(),
            server_key_file.path().to_str().unwrap(),
        );

        // ASSERT
        assert!(
            result.is_ok(),
            "Should load TLS config from valid paths: {:?}",
            result.err()
        );
    }

    #[test]
    fn load_tls_config_with_missing_ca() {
        // ARRANGE
        let ca_path = "/nonexistent/ca.crt";
        let server_cert_path = "/nonexistent/server.crt";
        let server_key_path = "/nonexistent/server.key";

        // ACT
        let result = load_tls_config_with_paths(ca_path, server_cert_path, server_key_path);

        // ASSERT
        assert!(result.is_err(), "Should fail with missing CA cert");
        match result {
            Err(e) => {
                assert!(
                    e.to_string().contains("CA certificate"),
                    "Error should mention CA certificate, got: {e}"
                );
            }
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }
}
