//! Certificate generation functions.

use core::str::FromStr as _;
use core::time::Duration;

use der::Encode as _;
use ring::digest::{SHA256, digest};
use x509_cert::Certificate;
use x509_cert::builder::{Builder as _, CertificateBuilder};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

use crate::error::{PkiError, Result};
use crate::hex::encode_lower;
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
        .map(|cert_der| {
            let digest = digest(&SHA256, &cert_der);
            encode_lower(digest.as_ref())
        })
        .map_err(PkiError::from)
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
