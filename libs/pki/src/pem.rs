//! PEM and PKCS#8 conversion helpers.

use der::{DecodePem as _, EncodePem as _, pem::LineEnding};
use x509_cert::Certificate;

use crate::error::{PkiError, Result};
use crate::key::Signer;

/// Encodes a certificate as PEM.
///
/// # Errors
///
/// Returns an error if the certificate cannot be DER-encoded for PEM wrapping.
pub fn encode_cert(cert: &Certificate) -> Result<String> {
    cert.to_pem(LineEnding::LF).map_err(PkiError::from)
}

/// Decodes a PEM-encoded certificate.
///
/// # Errors
///
/// Returns an error if the PEM document does not contain a valid certificate.
pub fn decode_cert(pem: &str) -> Result<Certificate> {
    Certificate::from_pem(pem).map_err(PkiError::from)
}

/// Encodes PKCS#8 DER as PEM.
///
/// # Errors
///
/// Returns an error if the DER bytes cannot be wrapped into a PEM document.
pub fn encode_pkcs8(der: &[u8]) -> Result<String> {
    der::SecretDocument::try_from(der)
        .and_then(|doc| doc.to_pem("PRIVATE KEY", LineEnding::LF))
        .map(|pem| pem.to_string())
        .map_err(PkiError::from)
}

/// Decodes PEM-encoded PKCS#8 into DER.
///
/// # Errors
///
/// Returns an error if the PEM document cannot be parsed.
pub fn decode_pkcs8(pem: &str) -> Result<Vec<u8>> {
    der::SecretDocument::from_pem(pem)
        .map(|(_label, doc)| doc.as_bytes().to_vec())
        .map_err(PkiError::from)
}

/// Loads a signer from a PEM-encoded PKCS#8 private key.
///
/// # Errors
///
/// Returns an error if the PEM cannot be parsed or the private key is invalid.
pub fn load_signer(pem: &str) -> Result<Signer> {
    let der = decode_pkcs8(pem)?;

    Signer::from_pkcs8_der(&der)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_private_key_pem_is_rejected() {
        // ARRANGE
        let invalid_pem = "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----\n";

        // ACT
        let decoded = decode_pkcs8(invalid_pem);
        let loaded = load_signer(invalid_pem);

        // ASSERT
        let _decoded_error = decoded.unwrap_err();
        let _loaded_error = loaded.map(|_| ()).unwrap_err();
    }
}
