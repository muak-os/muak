//! Certificate generation functions.

use core::str::FromStr as _;
use core::time::Duration;

use base16ct::lower::encode_string;
use der::Encode as _;
use sha2::{Digest as _, Sha256};
use x509_cert::Certificate;
use x509_cert::builder::{Builder as _, CertificateBuilder};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

use crate::error::{PkiError, Result};
use crate::key::{Signature, Signer};
use crate::profile::{MuakCa, MuakServer};
use crate::serial::{generate as generate_serial, signer_spki};

/// Certificate validity period (99 years).
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

struct CertificateMaterial {
    signer: Signer,
    serial: SerialNumber,
    validity: Validity,
    spki: spki::SubjectPublicKeyInfoOwned,
}

/// Generates a self-signed CA certificate.
///
/// # Errors
///
/// Returns an error if key generation, subject parsing, serial generation,
/// validity construction, SPKI encoding, or certificate building fails.
pub fn generate_ca(cn: &str) -> Result<(Signer, Certificate)> {
    let subject = Name::from_str(&format!("CN={cn},O=Muak"))?;
    let CertificateMaterial {
        signer,
        serial,
        validity,
        spki,
    } = certificate_material()?;

    ca_certificate(subject, serial, validity, spki, &signer).map(|cert| (signer, cert))
}

/// Generates a server certificate signed by the CA with SANs.
///
/// # Errors
///
/// Returns an error if key generation, subject parsing, serial generation,
/// validity construction, SPKI encoding, or certificate building fails.
pub fn generate_server(
    cn: &str,
    ca_signer: &Signer,
    ca_cert: &Certificate,
) -> Result<(Signer, Certificate)> {
    let subject = Name::from_str(&format!("CN={cn},O=Muak"))?;
    let issuer = ca_cert.tbs_certificate().subject().clone();
    let CertificateMaterial {
        signer,
        serial,
        validity,
        spki,
    } = certificate_material()?;

    server_certificate(cn, issuer, subject, serial, validity, spki, ca_signer)
        .map(|cert| (signer, cert))
}

/// Computes SHA256 fingerprint of a certificate (lowercase hex).
///
/// # Errors
///
/// Returns an error if DER encoding the certificate fails.
pub fn compute_fingerprint(cert: &Certificate) -> Result<String> {
    cert.to_der()
        .map(|cert_der| compute_fingerprint_der(&cert_der))
        .map_err(PkiError::from)
}

/// Computes SHA256 fingerprint of DER-encoded certificate bytes (lowercase hex).
#[must_use]
pub fn compute_fingerprint_der(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);

    encode_string(digest.as_ref())
}

fn certificate_validity() -> Result<Validity> {
    Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS)).map_err(PkiError::from)
}

fn certificate_material() -> Result<CertificateMaterial> {
    let signer = Signer::generate()?;
    let serial = generate_serial()?;
    let validity = certificate_validity()?;
    let spki = signer_spki(&signer)?;

    Ok(CertificateMaterial {
        signer,
        serial,
        validity,
        spki,
    })
}

fn ca_certificate(
    subject: Name,
    serial: SerialNumber,
    validity: Validity,
    spki: spki::SubjectPublicKeyInfoOwned,
    signer: &Signer,
) -> Result<Certificate> {
    let profile = MuakCa { subject };

    CertificateBuilder::new(profile, serial, validity, spki)
        .and_then(|builder| builder.build::<_, Signature>(signer))
        .map_err(PkiError::from)
}

fn server_certificate(
    cn: &str,
    issuer: Name,
    subject: Name,
    serial: SerialNumber,
    validity: Validity,
    spki: spki::SubjectPublicKeyInfoOwned,
    signer: &Signer,
) -> Result<Certificate> {
    let profile = MuakServer {
        issuer,
        subject,
        dns_names: vec![cn.to_owned(), "localhost".to_owned()],
    };

    CertificateBuilder::new(profile, serial, validity, spki)
        .and_then(|builder| builder.build::<_, Signature>(signer))
        .map_err(PkiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_der_is_64_lowercase_hex() {
        // ARRANGE
        let cert_der = [0xAB_u8, 0xCD, 0xEF];

        // ACT
        let fingerprint = compute_fingerprint_der(&cert_der);

        // ASSERT
        assert_eq!(fingerprint.len(), 64);
        assert!(
            fingerprint
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "Fingerprint should be lowercase hex: {fingerprint}"
        );
    }

    #[test]
    fn fingerprint_der_is_deterministic() {
        // ARRANGE
        let cert_der = b"same certificate bytes";

        // ACT
        let first = compute_fingerprint_der(cert_der);
        let second = compute_fingerprint_der(cert_der);

        // ASSERT
        assert_eq!(first, second, "Same input should produce same fingerprint");
    }

    #[test]
    fn fingerprint_der_distinguishes_inputs() {
        // ARRANGE
        let first_der = b"first certificate";
        let second_der = b"second certificate";

        // ACT
        let first = compute_fingerprint_der(first_der);
        let second = compute_fingerprint_der(second_der);

        // ASSERT
        assert_ne!(
            first, second,
            "Different inputs should produce different fingerprints"
        );
    }

    #[test]
    fn fingerprint_der_empty_input() {
        // ACT
        let fingerprint = compute_fingerprint_der(&[]);

        // ASSERT
        assert_eq!(
            fingerprint,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn fingerprint_der_matches_certificate_fingerprint() {
        // ARRANGE
        let (_, cert) = generate_ca("Test CA").expect("Failed to generate test CA");
        let cert_der = cert.to_der().expect("Failed to encode certificate to DER");

        // ACT
        let from_der = compute_fingerprint_der(&cert_der);
        let from_cert = compute_fingerprint(&cert).expect("Failed to compute fingerprint");

        // ASSERT
        assert_eq!(from_der, from_cert);
    }
}
