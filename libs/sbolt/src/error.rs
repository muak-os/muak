//! Error types for `sbolt` operations.

use thiserror::Error;

/// Error type for `sbolt` operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum SboltError {
    #[error("key generation failed: {0}")]
    KeyGeneration(String),

    #[error("certificate creation failed: {0}")]
    CertificateCreation(String),

    #[error("signing failed: {0}")]
    Signing(String),

    #[error("PE operation failed: {0}")]
    PeOperation(String),

    #[error("authenticode hash failed: {0}")]
    AuthenticodeHash(String),

    #[error("efivar operation failed: {0}")]
    EfiVar(String),

    #[error("key storage failed: {0}")]
    KeyStorage(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("DER error: {0}")]
    Der(#[from] der::Error),

    #[error("SPKI error: {0}")]
    Spki(#[from] spki::Error),
}

/// Result type alias for `sbolt` operations.
pub type Result<T> = core::result::Result<T, SboltError>;
