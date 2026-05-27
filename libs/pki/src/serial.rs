//! Serial and SPKI helpers for certificate construction.

use der::Decode as _;
use ring::rand::{SystemRandom, generate as generate_random};
use signature::Keypair as _;
use spki::SubjectPublicKeyInfoOwned;
use x509_cert::serial_number::SerialNumber;

use crate::error::{PkiError, Result};
use crate::key::Signer;

/// Extracts `SubjectPublicKeyInfo` from a signer.
///
/// # Errors
///
/// Returns an error if the signer's verifying key cannot be encoded as SPKI.
pub fn signer_spki(signer: &Signer) -> Result<SubjectPublicKeyInfoOwned> {
    use spki::EncodePublicKey as _;

    let verifying_key = signer.verifying_key();
    let der = verifying_key.to_public_key_der()?;

    Ok(SubjectPublicKeyInfoOwned::from_der(der.as_bytes())?)
}

/// Generates a random serial number.
///
/// # Errors
///
/// Returns an error if random byte generation fails or if the resulting serial
/// number is invalid.
pub fn generate() -> Result<SerialNumber> {
    let rng = SystemRandom::new();
    let random: [u8; 16] = generate_random(&rng)
        .map_err(|_random_error| PkiError::Random)?
        .expose();

    SerialNumber::new(&random).map_err(|_serial_error| PkiError::SerialNumber)
}
