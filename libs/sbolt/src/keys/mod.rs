//! Key generation and management.

pub mod cert;
pub mod hierarchy;
mod profile;
pub mod rsa2048;
pub mod storage;

use x509_cert::Certificate;

/// References to the key material needed for Authenticode PE signing.
pub struct SigningPair<'a> {
    /// RSA-2048 private key signer.
    pub signer: &'a rsa2048::Signer,
    /// X.509 certificate whose public key corresponds to `signer`.
    pub certificate: &'a Certificate,
}
