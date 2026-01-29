//! Certificate Signing Request (CSR) handling.

use der::{DecodePem, Encode, EncodePem, pem::LineEnding};
use std::str::FromStr;
use std::time::Duration;
use x509_cert::{
    Certificate,
    builder::{Builder, CertificateBuilder},
    name::Name,
    request::{CertReq, RequestBuilder},
    time::Validity,
};

use crate::cert::{CERT_VALIDITY_SECS, compute_cert_fingerprint};
use crate::error::{Error, Result};
use crate::profile::MuakClientProfile;
use crate::signer::{EcdsaSignature, RingEcdsaSigner};
use crate::util::{generate_serial, load_signer_from_pem};

/// Generates a CSR (Certificate Signing Request) for client authentication.
///
/// Returns (private_key_pem, csr_pem).
pub fn generate_csr(cn: &str) -> Result<(String, String)> {
    let signer = RingEcdsaSigner::generate()?;
    let subject = Name::from_str(&format!("CN={},O=Muak", cn))
        .map_err(|e| Error::InvalidName(e.to_string()))?;

    let builder = RequestBuilder::new(subject)?;
    let csr = builder.build::<_, EcdsaSignature>(&signer)?;

    let key_pem = crate::util::pkcs8_to_pem(signer.pkcs8_der())?;
    let csr_pem = csr.to_pem(LineEnding::LF)?;

    Ok((key_pem, csr_pem))
}

/// Signs a CSR with the CA and returns a client certificate.
///
/// Returns the signed certificate and its fingerprint.
pub fn sign_csr(
    csr_pem: &str,
    ca_key_pem: &str,
    ca_cert: &Certificate,
) -> Result<(Certificate, String)> {
    let csr = CertReq::from_pem(csr_pem)?;
    let ca_signer = load_signer_from_pem(ca_key_pem)?;

    let subject = csr.info.subject.clone();
    let spki = csr.info.public_key.clone();

    let serial = generate_serial()?;
    let validity =
        Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS)).map_err(|_| Error::Validity)?;

    let profile = MuakClientProfile {
        issuer: ca_cert.tbs_certificate().subject().clone(),
        subject,
    };

    let builder = CertificateBuilder::new(profile, serial, validity, spki)?;

    let cert = builder.build::<_, EcdsaSignature>(&ca_signer)?;
    let fingerprint = compute_cert_fingerprint(&cert)?;

    Ok((cert, fingerprint))
}

/// Computes SHA256 fingerprint of a CSR's public key (plain hex, no colons).
///
/// This is used as the identifier for pending CSRs.
pub fn compute_csr_fingerprint(csr_pem: &str) -> Result<String> {
    let csr = CertReq::from_pem(csr_pem)?;
    let spki_der = csr.info.public_key.to_der()?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &spki_der);
    Ok(hex::encode(digest.as_ref()))
}
