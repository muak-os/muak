//! Certificate generation functions.

use der::Encode;
use std::str::FromStr;
use std::time::Duration;
use x509_cert::{
    Certificate,
    builder::{Builder, CertificateBuilder},
    name::Name,
    time::Validity,
};

use crate::error::{Error, Result};
use crate::profile::{MuakCaProfile, MuakServerProfile};
use crate::signer::{EcdsaSignature, RingEcdsaSigner};
use crate::util::{generate_serial, get_spki_from_signer};

/// Certificate validity period (99 years)
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

/// Generates a self-signed CA certificate.
pub fn generate_ca_certificate(cn: &str) -> Result<(RingEcdsaSigner, Certificate)> {
    let signer = RingEcdsaSigner::generate()?;

    let serial = generate_serial()?;
    let validity =
        Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS)).map_err(|_| Error::Validity)?;
    let subject = Name::from_str(&format!("CN={},O=Muak", cn))
        .map_err(|e| Error::InvalidName(e.to_string()))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = MuakCaProfile { subject };
    let builder = CertificateBuilder::new(profile, serial, validity, spki)?;

    let cert = builder.build::<_, EcdsaSignature>(&signer)?;

    Ok((signer, cert))
}

/// Generates a server certificate signed by the CA with SANs.
pub fn generate_server_certificate(
    cn: &str,
    ca_signer: &RingEcdsaSigner,
    ca_cert: &Certificate,
) -> Result<(RingEcdsaSigner, Certificate)> {
    let signer = RingEcdsaSigner::generate()?;

    let serial = generate_serial()?;
    let validity =
        Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS)).map_err(|_| Error::Validity)?;
    let subject = Name::from_str(&format!("CN={},O=Muak", cn))
        .map_err(|e| Error::InvalidName(e.to_string()))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = MuakServerProfile {
        issuer: ca_cert.tbs_certificate().subject().clone(),
        subject,
        dns_names: vec![cn.to_string(), "localhost".to_string()],
    };

    let builder = CertificateBuilder::new(profile, serial, validity, spki)?;

    let cert = builder.build::<_, EcdsaSignature>(ca_signer)?;

    Ok((signer, cert))
}

/// Computes SHA256 fingerprint of a certificate (plain hex, no colons).
pub fn compute_cert_fingerprint(cert: &Certificate) -> Result<String> {
    let cert_der = cert.to_der()?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &cert_der);
    Ok(hex::encode(digest.as_ref()))
}
