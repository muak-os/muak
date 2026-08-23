//! Key generation and management.

use std::path::Path;

use x509_cert::Certificate;

use crate::error::{Result, SboltError};

pub mod cert;
pub mod hierarchy;
mod profile;
pub mod rsa2048;
pub mod storage;

/// References to the key material needed for Authenticode PE signing.
pub struct SigningPair<'a> {
    /// RSA-2048 private key signer.
    pub signer: &'a rsa2048::Signer,
    /// X.509 certificate whose public key corresponds to `signer`.
    pub certificate: &'a Certificate,
}

/// Load an RSA-2048 signer from a PEM-encoded PKCS#8 private key file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the key cannot be parsed.
pub fn load_signer_from_pem(path: &Path) -> Result<rsa2048::Signer> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| SboltError::KeyStorage(format!("read {}: {e}", path.display())))?;
    let der = storage::pem_to_pkcs8_der(&pem)?;

    rsa2048::Signer::from_pkcs8_der(&der)
}

/// Load an X.509 certificate from a PEM-encoded certificate file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the certificate cannot be
/// parsed.
pub fn load_certificate_from_pem(path: &Path) -> Result<Certificate> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| SboltError::KeyStorage(format!("read {}: {e}", path.display())))?;

    storage::pem_to_cert(&pem)
}
