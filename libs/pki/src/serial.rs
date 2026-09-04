//! Serial and SPKI helpers for certificate construction.

use signature::Keypair as _;
use spki::SubjectPublicKeyInfoOwned;
use x509_cert::serial_number::SerialNumber;

use crate::error::{PkiError, Result};
use crate::key::Signer;

/// Extracts `SubjectPublicKeyInfo` from a signer.
///
/// # Errors
///
/// Returns an error if the signer's public key cannot be encoded as SPKI.
pub fn signer_spki(signer: &Signer) -> Result<SubjectPublicKeyInfoOwned> {
    signer
        .verifying_key()
        .subject_public_key_info()
        .map_err(PkiError::from)
}

/// Generates a random serial number.
///
/// # Errors
///
/// Returns an error if random byte generation fails or if the resulting serial
/// number is invalid.
pub fn generate() -> Result<SerialNumber> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_random_error| PkiError::Random)?;

    SerialNumber::new(&random).map_err(|_serial_error| PkiError::SerialNumber)
}
