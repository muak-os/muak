//! Certificate generation functions.

use core::str::FromStr as _;
use core::time::Duration;

use der::Encode as _;
use ring::digest::{SHA256, digest};
use x509_cert::{
    Certificate,
    builder::{Builder as _, CertificateBuilder},
    name::Name,
    time::Validity,
};

use crate::error::Result;
use crate::hex::encode_lower;
use crate::key::{Signature, Signer};
use crate::profile::{MuakCa, MuakServer};
use crate::serial::{generate as generate_serial, signer_spki};

/// Certificate validity period (99 years).
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

/// Generates a self-signed CA certificate.
///
/// # Errors
///
/// Returns an error if key generation, subject parsing, serial generation,
/// validity construction, SPKI encoding, or certificate building fails.
pub fn generate_ca(cn: &str) -> Result<(Signer, Certificate)> {
    let signer = Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak"))?;

    let spki = signer_spki(&signer)?;

    let profile = MuakCa { subject };
    let builder = CertificateBuilder::new(profile, serial, validity, spki)?;

    let cert = builder.build::<_, Signature>(&signer)?;

    Ok((signer, cert))
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
    let signer = Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak"))?;

    let spki = signer_spki(&signer)?;

    let profile = MuakServer {
        issuer: ca_cert.tbs_certificate().subject().clone(),
        subject,
        dns_names: vec![cn.to_owned(), "localhost".to_owned()],
    };

    let builder = CertificateBuilder::new(profile, serial, validity, spki)?;

    let cert = builder.build::<_, Signature>(ca_signer)?;

    Ok((signer, cert))
}

/// Computes SHA256 fingerprint of a certificate (lowercase hex).
///
/// # Errors
///
/// Returns an error if DER encoding the certificate fails.
pub fn compute_fingerprint(cert: &Certificate) -> Result<String> {
    let cert_der = cert.to_der()?;
    let digest = digest(&SHA256, &cert_der);
    Ok(encode_lower(digest.as_ref()))
}
