//! Certificate generation for Secure Boot keys

use std::str::FromStr;
use std::time::Duration;

use der::Decode;
use ring::rand::SystemRandom;
use signature::Keypair;
use spki::{EncodePublicKey, SubjectPublicKeyInfoOwned};
use x509_cert::{
    Certificate,
    builder::{Builder, CertificateBuilder},
    name::Name,
    serial_number::SerialNumber,
    time::Validity,
};

use super::profile::SecureBootProfile;
use super::signer::{Rsa2048Signature, Rsa2048Signer};
use crate::{Error, Result};

/// Certificate validity period (99 years)
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

/// Generates a self-signed Platform Key (PK) certificate
pub fn generate_pk_certificate(cn: &str) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| Error::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={},O=Muak Secure Boot", cn))
        .map_err(|e| Error::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::pk(subject);
    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(&signer)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generates a Key Exchange Key (KEK) certificate signed by PK
pub fn generate_kek_certificate(
    cn: &str,
    pk_signer: &Rsa2048Signer,
    pk_cert: &Certificate,
) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| Error::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={},O=Muak Secure Boot", cn))
        .map_err(|e| Error::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::kek(pk_cert.tbs_certificate().subject().clone(), subject);

    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(pk_signer)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generates a Signature Database (db) certificate signed by KEK
pub fn generate_db_certificate(
    cn: &str,
    kek_signer: &Rsa2048Signer,
    kek_cert: &Certificate,
) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| Error::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={},O=Muak Secure Boot", cn))
        .map_err(|e| Error::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::db(kek_cert.tbs_certificate().subject().clone(), subject);

    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(kek_signer)
        .map_err(|e| Error::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generates a random 128-bit serial number
pub fn generate_serial() -> Result<SerialNumber> {
    let rng = SystemRandom::new();
    let random: [u8; 16] = ring::rand::generate(&rng)
        .map_err(|_| Error::KeyGeneration("failed to generate random serial".into()))?
        .expose();
    SerialNumber::new(&random)
        .map_err(|e| Error::CertificateCreation(format!("invalid serial: {e}")))
}

/// Extracts SubjectPublicKeyInfo from a signer
pub fn get_spki_from_signer(signer: &Rsa2048Signer) -> Result<SubjectPublicKeyInfoOwned> {
    let verifying_key = signer.verifying_key();
    let der = verifying_key.to_public_key_der()?;
    Ok(SubjectPublicKeyInfoOwned::from_der(der.as_bytes())?)
}
