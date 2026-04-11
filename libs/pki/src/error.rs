//! PKI error types.

use thiserror::Error;

/// PKI error types.
#[derive(Debug, Error)]
pub enum Error {
    #[error("key generation failed")]
    KeyGeneration,

    #[error("invalid key encoding")]
    InvalidKeyEncoding,

    #[error("certificate building failed: {0}")]
    CertificateBuild(#[from] x509_cert::builder::Error),

    #[error("DER encoding failed: {0}")]
    DerEncode(#[from] der::Error),

    #[error("SPKI error: {0}")]
    Spki(#[from] spki::Error),

    #[error("invalid name: {0}")]
    InvalidName(String),

    #[error("validity period error")]
    Validity,

    #[error("serial number generation failed")]
    SerialNumber,

    #[error("random generation failed")]
    Random,

    #[error("CSR signature verification failed")]
    CsrVerification,
}

/// Result type for PKI operations.
pub type Result<T> = std::result::Result<T, Error>;
