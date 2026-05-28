//! PKI error types.

use core::result;

use der::Error as DerError;
use thiserror::Error as ThisError;
use x509_cert::builder::Error as CertificateBuilderError;

/// Error type for custom errors in PKI operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, ThisError)]
pub enum PkiError {
    #[error("key generation failed")]
    KeyGeneration,

    #[error("invalid key encoding")]
    InvalidKeyEncoding,

    #[error("certificate building failed: {0}")]
    CertificateBuild(#[from] CertificateBuilderError),

    #[error("DER processing failed: {0}")]
    Der(#[from] DerError),

    #[error("SPKI error: {0}")]
    Spki(#[from] spki::Error),

    #[error("serial number generation failed")]
    SerialNumber,

    #[error("random generation failed")]
    Random,

    #[error("CSR signature verification failed")]
    CsrVerification,
}

/// Result type alias for PKI operations.
pub type Result<T> = result::Result<T, PkiError>;
