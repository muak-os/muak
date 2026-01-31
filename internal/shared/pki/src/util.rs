//! Utility functions for PEM/DER conversion and key operations.

use der::{Decode, pem::LineEnding};
use ring::rand::SystemRandom;
use signature::Keypair;
use spki::SubjectPublicKeyInfoOwned;
use x509_cert::serial_number::SerialNumber;

use crate::error::{Error, Result};
use crate::signer::RingEcdsaSigner;

/// Converts PKCS#8 DER to PEM format.
pub fn pkcs8_to_pem(der: &[u8]) -> Result<String> {
    let doc = der::SecretDocument::try_from(der)?;
    let pem = doc.to_pem("PRIVATE KEY", LineEnding::LF)?;
    Ok(pem.to_string())
}

/// Converts PEM-encoded PKCS#8 to DER.
pub fn pem_to_pkcs8_der(pem: &str) -> Result<Vec<u8>> {
    let (_label, doc) = der::SecretDocument::from_pem(pem)?;
    Ok(doc.as_bytes().to_vec())
}

/// Loads a signer from a PEM-encoded PKCS#8 private key.
pub fn load_signer_from_pem(pem: &str) -> Result<RingEcdsaSigner> {
    let der = pem_to_pkcs8_der(pem)?;
    RingEcdsaSigner::from_pkcs8_der(&der)
}

/// Extracts SubjectPublicKeyInfo from a signer.
pub fn get_spki_from_signer(signer: &RingEcdsaSigner) -> Result<SubjectPublicKeyInfoOwned> {
    use spki::EncodePublicKey;
    let verifying_key = signer.verifying_key();
    let der = verifying_key.to_public_key_der()?;
    Ok(SubjectPublicKeyInfoOwned::from_der(der.as_bytes())?)
}

/// Generates a random serial number.
pub fn generate_serial() -> Result<SerialNumber> {
    let rng = SystemRandom::new();
    let random: [u8; 16] = ring::rand::generate(&rng)
        .map_err(|_| Error::Random)?
        .expose();
    SerialNumber::new(&random).map_err(|_| Error::SerialNumber)
}

/// Converts a byte slice to a lowercase hex string.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}
