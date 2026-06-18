//! Error types for `sbolt` operations.

use thiserror::Error;

/// Error type for `sbolt` operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum SboltError {
    /// Key generation failed.
    #[error("key generation failed: {0}")]
    KeyGeneration(String),

    /// Certificate creation failed.
    #[error("certificate creation failed: {0}")]
    CertificateCreation(String),

    /// Signing operation failed.
    #[error("signing failed: {0}")]
    Signing(String),

    /// PE operation failed.
    #[error("PE operation failed: {0}")]
    PeOperation(String),

    /// EFI variable operation failed.
    #[error("efivar operation failed: {0}")]
    EfiVar(String),

    /// Key storage operation failed.
    #[error("key storage failed: {0}")]
    KeyStorage(String),

    /// I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// DER encoding or decoding error.
    #[error("DER error: {0}")]
    Der(#[from] der::Error),

    /// SPKI (`SubjectPublicKeyInfo`) error.
    #[error("SPKI error: {0}")]
    Spki(#[from] spki::Error),
}

/// Result type alias for `sbolt` operations.
pub type Result<T> = core::result::Result<T, SboltError>;
