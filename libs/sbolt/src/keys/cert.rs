//! Certificate generation for Secure Boot keys.

use core::str::FromStr as _;
use core::time::Duration;

use der::Decode as _;
use ring::rand::{SystemRandom, generate};
use signature::Keypair as _;
use spki::{EncodePublicKey as _, SubjectPublicKeyInfoOwned};
use x509_cert::Certificate;
use x509_cert::builder::{Builder as _, CertificateBuilder};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

use super::profile::SecureBootProfile;
use super::signer::{Rsa2048Signature, Rsa2048Signer};
use crate::error::{Result, SboltError};

/// Certificate validity period (99 years).
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

/// Generate a self-signed Platform Key (PK) certificate.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_pk_certificate(cn: &str) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| SboltError::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak Secure Boot"))
        .map_err(|e| SboltError::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::pk(subject);
    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(&signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generate a Key Exchange Key (KEK) certificate signed by PK.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_kek_certificate(
    cn: &str,
    pk_signer: &Rsa2048Signer,
    pk_cert: &Certificate,
) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| SboltError::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak Secure Boot"))
        .map_err(|e| SboltError::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::kek(pk_cert.tbs_certificate().subject().clone(), subject);

    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(pk_signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generate a Signature Database (db) certificate signed by KEK.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_db_certificate(
    cn: &str,
    kek_signer: &Rsa2048Signer,
    kek_cert: &Certificate,
) -> Result<(Rsa2048Signer, Certificate)> {
    let signer = Rsa2048Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| SboltError::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak Secure Boot"))
        .map_err(|e| SboltError::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::db(kek_cert.tbs_certificate().subject().clone(), subject);

    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, Rsa2048Signature>(kek_signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

fn generate_serial() -> Result<SerialNumber> {
    let rng = SystemRandom::new();
    let random: [u8; 16] = generate(&rng)
        .map_err(|_random_error| {
            SboltError::KeyGeneration("failed to generate random serial".into())
        })?
        .expose();
    SerialNumber::new(&random)
        .map_err(|e| SboltError::CertificateCreation(format!("invalid serial: {e}")))
}

fn get_spki_from_signer(signer: &Rsa2048Signer) -> Result<SubjectPublicKeyInfoOwned> {
    let verifying_key = signer.verifying_key();
    let der = verifying_key.to_public_key_der()?;
    Ok(SubjectPublicKeyInfoOwned::from_der(der.as_bytes())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_pk_certificate_produces_self_signed_ca() -> Result<()> {
        // ARRANGE
        let common_name = "Platform";

        // ACT
        let (_signer, certificate) = generate_pk_certificate(common_name)?;

        // ASSERT
        assert_eq!(
            certificate.tbs_certificate().issuer(),
            certificate.tbs_certificate().subject()
        );

        Ok(())
    }

    #[test]
    fn generate_kek_and_db_certificates_chain_subjects() -> Result<()> {
        // ARRANGE
        let (pk_signer, pk_cert) = generate_pk_certificate("PK")?;

        // ACT
        let (_kek_signer, kek_cert) = generate_kek_certificate("KEK", &pk_signer, &pk_cert)?;
        let (_db_signer, db_cert) = generate_db_certificate("DB", &pk_signer, &pk_cert)?;

        // ASSERT
        assert_eq!(
            kek_cert.tbs_certificate().issuer(),
            pk_cert.tbs_certificate().subject()
        );
        assert_eq!(
            db_cert.tbs_certificate().issuer(),
            pk_cert.tbs_certificate().subject()
        );

        Ok(())
    }

    #[test]
    fn generate_serial_and_spki_are_non_empty() -> Result<()> {
        // ARRANGE
        let signer = Rsa2048Signer::generate()?;

        // ACT
        let serial = generate_serial()?;
        let spki = get_spki_from_signer(&signer)?;

        // ASSERT
        assert!(!serial.as_bytes().is_empty());
        assert!(!spki.subject_public_key.raw_bytes().is_empty());

        Ok(())
    }
}
