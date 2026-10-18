//! Certificate generation for Secure Boot keys.

use core::str::FromStr as _;
use core::time::Duration;

use signature::Keypair as _;
use spki::SubjectPublicKeyInfoOwned;
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

/// Generate a self-signed Signature Database (db) certificate.
///
/// # Errors
///
/// Returns an error if key generation, subject construction, or certificate
/// building fails.
pub fn generate_db(cn: &str) -> Result<(rsa2048::Signer, Certificate)> {
    let signer = rsa2048::Signer::generate()?;

    let serial = generate_serial()?;
    let validity = Validity::from_now(Duration::from_secs(CERT_VALIDITY_SECS))
        .map_err(|e| SboltError::CertificateCreation(format!("validity: {e}")))?;
    let subject = Name::from_str(&format!("CN={cn},O=Muak Secure Boot"))
        .map_err(|e| SboltError::CertificateCreation(format!("name: {e}")))?;

    let spki = get_spki_from_signer(&signer)?;

    let profile = SecureBootProfile::db(subject);
    let builder = CertificateBuilder::new(profile, serial, validity, spki)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    let cert = builder
        .build::<_, rsa2048::Signature>(&signer)
        .map_err(|e| SboltError::CertificateCreation(e.to_string()))?;

    Ok((signer, cert))
}

fn generate_serial() -> Result<SerialNumber> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_random_error| {
        SboltError::KeyGeneration("failed to generate random serial".into())
    })?;

    SerialNumber::new(&random)
        .map_err(|e| SboltError::CertificateCreation(format!("invalid serial: {e}")))
}

fn get_spki_from_signer(signer: &rsa2048::Signer) -> Result<SubjectPublicKeyInfoOwned> {
    SubjectPublicKeyInfoOwned::from_key(&signer.verifying_key())
        .map_err(|e| SboltError::CertificateCreation(format!("SPKI encoding failed: {e}")))
}

#[cfg(test)]
mod tests {
    use const_oid::db::rfc5280::ID_KP_CODE_SIGNING;
    use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage};

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
    fn generate_kek_certificate_is_issued_by_pk() {
        // ARRANGE
        let (pk_signer, pk_cert) = generate_pk("PK").expect("generate PK certificate");

        // ACT
        let (_kek_signer, kek_cert) =
            generate_kek("KEK", &pk_signer, &pk_cert).expect("generate KEK certificate");

        // ASSERT
        assert_eq!(
            kek_cert.tbs_certificate().issuer(),
            pk_cert.tbs_certificate().subject()
        );
    }

    #[test]
    fn generate_db_certificate_is_a_self_signed_trust_anchor() {
        // ARRANGE
        let (_signer, certificate) = generate_db("DB").expect("generate db certificate");

        // ACT
        let subject = certificate.tbs_certificate().subject();
        let issuer = certificate.tbs_certificate().issuer();
        let (_, constraints) = certificate
            .tbs_certificate()
            .get_extension::<BasicConstraints>()
            .expect("read basic constraints")
            .expect("basic constraints extension");

        // ASSERT
        assert_eq!(issuer, subject, "db must be self-signed");
        assert!(constraints.ca, "db must be a CA trust anchor");
    }

    #[test]
    fn generate_db_certificate_includes_code_signing_usage() {
        // ARRANGE
        let (_signer, certificate) = generate_db("DB").expect("generate db certificate");

        // ACT
        let (_, usage) = certificate
            .tbs_certificate()
            .get_extension::<ExtendedKeyUsage>()
            .expect("read extended key usage")
            .expect("extended key usage extension");

        // ASSERT
        assert!(
            usage.0.contains(&ID_KP_CODE_SIGNING),
            "db must allow code signing"
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
        assert_ne!(serial.as_bytes(), []);
        assert_ne!(spki.subject_public_key.raw_bytes(), []);
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
