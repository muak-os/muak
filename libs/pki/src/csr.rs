//! Certificate Signing Request (CSR) handling.

use core::str::FromStr as _;
use core::time::Duration;

use der::{DecodePem as _, Encode as _, EncodePem as _, pem::LineEnding};
use ring::digest::{SHA256, digest};
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use x509_cert::Certificate;
use x509_cert::builder::{Builder as _, CertificateBuilder};
use x509_cert::name::Name;
use x509_cert::request::{CertReq, RequestBuilder};
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

use crate::cert::{self, CERT_VALIDITY_SECS};
use crate::error::{PkiError, Result};
use crate::hex::encode_lower;
use crate::key::{Signature, Signer};
use crate::pem::{encode_pkcs8, load_signer};
use crate::profile::MuakClient;
use crate::serial::generate as generate_serial;

/// Generates a CSR (Certificate Signing Request) for client authentication.
///
/// # Errors
///
/// Returns an error if key generation, subject parsing, CSR building, or PEM
/// encoding fails.
pub fn generate(cn: &str) -> Result<(String, String)> {
    let subject = Name::from_str(&format!("CN={cn},O=Muak"))?;
    let signer = Signer::generate()?;
    let csr = build_csr(subject, &signer)?;
    let key_pem = encode_pkcs8(signer.pkcs8_der())?;
    let csr_pem = csr.to_pem(LineEnding::LF)?;

    Ok((key_pem, csr_pem))
}

/// Signs a CSR with the CA and returns a client certificate.
///
/// # Errors
///
/// Returns an error if CSR parsing or verification fails, if the CA key cannot
/// be loaded, or if certificate building and fingerprinting fails.
pub fn sign(
    csr_pem: &str,
    ca_key_pem: &str,
    ca_cert: &Certificate,
) -> Result<(Certificate, String)> {
    let csr = CertReq::from_pem(csr_pem)?;

    verify_signature(&csr)?;

    let subject = csr.info.subject.clone();
    let spki = csr.info.public_key.clone();
    let issuer = ca_cert.tbs_certificate().subject().clone();
    let ca_signer = load_signer(ca_key_pem)?;
    let serial = generate_serial()?;
    let validity = certificate_validity()?;
    let cert = client_certificate(issuer, subject, serial, validity, spki, &ca_signer)?;
    let fingerprint = cert::compute_fingerprint(&cert)?;

    Ok((cert, fingerprint))
}

/// Verifies the self-signature on a CSR.
fn verify_signature(csr: &CertReq) -> Result<()> {
    let info_der = csr.info.to_der().map_err(PkiError::from)?;

    let pub_key_der = csr
        .info
        .public_key
        .subject_public_key
        .as_bytes()
        .ok_or(PkiError::CsrVerification)?;

    let sig_bytes = csr.signature.as_bytes().ok_or(PkiError::CsrVerification)?;

    let public_key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, pub_key_der);

    public_key
        .verify(&info_der, sig_bytes)
        .map_err(|_verification_error| PkiError::CsrVerification)
}

/// Computes SHA256 fingerprint of a CSR's public key (lowercase hex).
///
/// # Errors
///
/// Returns an error if CSR parsing or SPKI DER encoding fails.
pub fn compute_fingerprint(csr_pem: &str) -> Result<String> {
    let csr = CertReq::from_pem(csr_pem)?;
    let spki_der = csr.info.public_key.to_der()?;
    let digest = digest(&SHA256, &spki_der);

    Ok(encode_lower(digest.as_ref()))
}

fn build_csr(subject: Name, signer: &Signer) -> Result<CertReq> {
    RequestBuilder::new(subject)
        .and_then(|builder| builder.build::<_, Signature>(signer))
        .map_err(PkiError::from)
}

fn certificate_validity() -> Result<Validity> {
    Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS)).map_err(PkiError::from)
}

fn client_certificate(
    issuer: Name,
    subject: Name,
    serial: SerialNumber,
    validity: Validity,
    spki: spki::SubjectPublicKeyInfoOwned,
    signer: &Signer,
) -> Result<Certificate> {
    let profile = MuakClient { issuer, subject };
    CertificateBuilder::new(profile, serial, validity, spki)
        .and_then(|builder| builder.build::<_, Signature>(signer))
        .map_err(PkiError::from)
}
