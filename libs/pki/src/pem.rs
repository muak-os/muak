//! PEM and PKCS#8 conversion helpers.

use der::pem::LineEnding;

use crate::error::Result;
use crate::key::Signer;

/// Encodes PKCS#8 DER as PEM.
///
/// # Errors
///
/// Returns an error if the DER bytes cannot be wrapped into a PEM document.
pub fn encode_pkcs8(der: &[u8]) -> Result<String> {
    let doc = der::SecretDocument::try_from(der)?;
    let pem = doc.to_pem("PRIVATE KEY", LineEnding::LF)?;

    Ok(pem.to_string())
}

/// Decodes PEM-encoded PKCS#8 into DER.
///
/// # Errors
///
/// Returns an error if the PEM document cannot be parsed.
pub fn decode_pkcs8(pem: &str) -> Result<Vec<u8>> {
    let (_label, doc) = der::SecretDocument::from_pem(pem)?;

    Ok(doc.as_bytes().to_vec())
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
