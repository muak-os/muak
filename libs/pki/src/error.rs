//! PKI error types.

use core::result;

use der::Error as DerError;
use thiserror::Error;
use x509_cert::builder::Error as CertificateBuilderError;

/// PKI error types.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum PkiError {
    #[error("key generation failed")]
    KeyGeneration,

    #[error("invalid key encoding")]
    InvalidKeyEncoding,

    #[error("certificate building failed: {0}")]
    CertificateBuild(#[from] CertificateBuilderError),

    #[error("DER encoding failed: {0}")]
    DerEncode(#[from] DerError),

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
pub type Result<T> = result::Result<T, PkiError>;
