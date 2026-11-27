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
    /// Key generation failed.
    #[error("key generation failed")]
    KeyGeneration,

    /// Invalid key encoding.
    #[error("invalid key encoding")]
    InvalidKeyEncoding,

    /// Certificate building failed.
    #[error("certificate building failed: {0}")]
    CertificateBuild(#[from] CertificateBuilderError),

    /// DER processing failed.
    #[error("DER processing failed: {0}")]
    Der(#[from] DerError),

    /// SPKI error.
    #[error("SPKI error: {0}")]
    Spki(#[from] spki::Error),

    /// Serial number generation failed.
    #[error("serial number generation failed")]
    SerialNumber,

    /// Random generation failed.
    #[error("random generation failed")]
    Random,

    /// CSR signature verification failed.
    #[error("CSR signature verification failed")]
    CsrVerification,
}

/// Result type alias for PKI operations.
pub type Result<T> = result::Result<T, PkiError>;
