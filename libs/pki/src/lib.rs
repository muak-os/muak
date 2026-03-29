//! PKI (Public Key Infrastructure) utilities for Muak mTLS authentication.
//!
//! This crate provides certificate generation, CSR handling, and fingerprint
//! computation for the Muak authentication system using ECDSA P-256.

mod cert;
mod csr;
mod error;
mod oid;
mod profile;
mod signer;
pub mod util;

pub use cert::{compute_cert_fingerprint, generate_ca_certificate, generate_server_certificate};
pub use csr::{compute_csr_fingerprint, generate_csr, sign_csr};
pub use error::{Error, Result};
pub use signer::RingEcdsaSigner;

#[cfg(test)]
mod tests {
    use der::{DecodePem, EncodePem, pem::LineEnding};
    use x509_cert::Certificate;

    use super::*;
    use crate::signer::RingEcdsaSigner;
    use crate::util::pkcs8_to_pem;

    fn make_test_ca() -> (String, String, RingEcdsaSigner, Certificate) {
        let (signer, cert) =
            generate_ca_certificate("Test CA").expect("Failed to generate test CA");
        let cert_pem = cert
            .to_pem(LineEnding::LF)
            .expect("Failed to encode CA cert");
        let key_pem = pkcs8_to_pem(signer.pkcs8_der()).expect("Failed to encode CA key");
        (key_pem, cert_pem, signer, cert)
    }

    #[test]
    fn generate_ca_and_server_cert() {
        // ARRANGE
        let (ca_key_pem, ca_cert_pem, ca_signer, ca_cert) = make_test_ca();

        // ACT
        let (server_key, server_cert) =
            generate_server_certificate("muak-server", &ca_signer, &ca_cert)
                .expect("Failed to generate server cert");
        let server_cert_pem = server_cert
            .to_pem(LineEnding::LF)
            .expect("Failed to encode server cert");
        let server_key_pem =
            pkcs8_to_pem(server_key.pkcs8_der()).expect("Failed to encode server key");

        let fingerprint =
            compute_cert_fingerprint(&server_cert).expect("Failed to compute fingerprint");

        // ASSERT
        assert!(ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(server_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(server_key_pem.contains("BEGIN PRIVATE KEY"));
        assert_eq!(fingerprint.len(), 64);
    }

    #[test]
    fn generate_and_sign_csr() {
        // ARRANGE
        let (ca_key_pem, _, _, ca_cert) = make_test_ca();

        let (key_pem, csr_pem) = generate_csr("test-client").expect("Failed to generate CSR");
        let csr_fp = compute_csr_fingerprint(&csr_pem).expect("Failed to compute CSR fingerprint");

        // ACT
        let (cert, cert_fp) =
            sign_csr(&csr_pem, &ca_key_pem, &ca_cert).expect("Failed to sign CSR");
        let cert_pem = cert.to_pem(LineEnding::LF).expect("Failed to encode cert");

        // ASSERT
        assert!(!key_pem.is_empty());
        assert!(!csr_pem.is_empty());
        assert_eq!(csr_fp.len(), 64);
        assert_eq!(cert_fp.len(), 64);
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn load_ca_from_pem() {
        // ARRANGE
        let (ca_key_pem, ca_cert_pem, _, _) = make_test_ca();
        let loaded_cert =
            Certificate::from_pem(&ca_cert_pem).expect("Failed to parse CA certificate");
        let (_, csr_pem) = generate_csr("test-client").expect("Failed to generate CSR");

        // ACT
        let (_, fp) =
            sign_csr(&csr_pem, &ca_key_pem, &loaded_cert).expect("Failed to sign with loaded CA");

        // ASSERT
        assert_eq!(fp.len(), 64);
    }
}
