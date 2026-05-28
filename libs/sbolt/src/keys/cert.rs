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
use super::rsa2048;
use crate::error::{Result, SboltError};

/// Certificate validity period (99 years).
pub const CERT_VALIDITY_SECS: u64 = 99 * 365 * 24 * 60 * 60;

/// Generate a self-signed Platform Key (PK) certificate.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_pk(cn: &str) -> Result<(rsa2048::Signer, Certificate)> {
    let signer = rsa2048::Signer::generate()?;

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
        .build::<_, rsa2048::Signature>(&signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generate a Key Exchange Key (KEK) certificate signed by PK.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_kek(
    cn: &str,
    pk_signer: &rsa2048::Signer,
    pk_cert: &Certificate,
) -> Result<(rsa2048::Signer, Certificate)> {
    let signer = rsa2048::Signer::generate()?;

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
        .build::<_, rsa2048::Signature>(pk_signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

/// Generate a Signature Database (db) certificate signed by KEK.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_db(
    cn: &str,
    kek_signer: &rsa2048::Signer,
    kek_cert: &Certificate,
) -> Result<(rsa2048::Signer, Certificate)> {
    let signer = rsa2048::Signer::generate()?;

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
        .build::<_, rsa2048::Signature>(kek_signer)
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

fn get_spki_from_signer(signer: &rsa2048::Signer) -> Result<SubjectPublicKeyInfoOwned> {
    let verifying_key = signer.verifying_key();
    let der = verifying_key.to_public_key_der()?;
    Ok(SubjectPublicKeyInfoOwned::from_der(der.as_bytes())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_pk_certificate_produces_self_signed_ca() {
        // ARRANGE
        let common_name = "Platform";

        // ACT
        let (_signer, certificate) = generate_pk(common_name).expect("generate PK certificate");

        // ASSERT
        assert_eq!(
            certificate.tbs_certificate().issuer(),
            certificate.tbs_certificate().subject()
        );
    }

    #[test]
    fn generate_kek_and_db_certificates_chain_subjects() {
        // ARRANGE
        let (pk_signer, pk_cert) = generate_pk("PK").expect("generate PK certificate");

        // ACT
        let (_kek_signer, kek_cert) =
            generate_kek("KEK", &pk_signer, &pk_cert).expect("generate KEK certificate");
        let (_db_signer, db_cert) =
            generate_db("DB", &pk_signer, &pk_cert).expect("generate db certificate");

        // ASSERT
        assert_eq!(
            kek_cert.tbs_certificate().issuer(),
            pk_cert.tbs_certificate().subject()
        );
        assert_eq!(
            db_cert.tbs_certificate().issuer(),
            pk_cert.tbs_certificate().subject()
        );
    }

    #[test]
    fn generate_serial_and_spki_are_non_empty() {
        // ARRANGE
        let signer = rsa2048::Signer::generate().expect("generate RSA signer");

        // ACT
        let serial = generate_serial().expect("generate serial");
        let spki = get_spki_from_signer(&signer).expect("get SPKI");

        // ASSERT
        assert!(!serial.as_bytes().is_empty());
        assert!(!spki.subject_public_key.raw_bytes().is_empty());
    }

    #[test]
    fn generated_certificates_include_common_names() {
        // ARRANGE
        let (pk_signer, pk_cert) = generate_pk("Platform Name").expect("generate PK certificate");

        // ACT
        let (_kek_signer, kek_cert) =
            generate_kek("Exchange Name", &pk_signer, &pk_cert).expect("generate KEK certificate");

        // ASSERT
        assert!(format!("{}", pk_cert.tbs_certificate().subject()).contains("Platform Name"));
        assert!(format!("{}", kek_cert.tbs_certificate().subject()).contains("Exchange Name"));
    }
}
