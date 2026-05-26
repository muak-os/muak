//! LUKS2 error types.

use thiserror::Error;

/// LUKS2 operation error types.
#[derive(Debug, Error)]
pub enum Luks2Error {
    #[error("invalid LUKS2 magic bytes")]
    InvalidMagic,

    #[error("unsupported LUKS version: {0}")]
    UnsupportedVersion(u16),

    #[error("header checksum mismatch")]
    ChecksumMismatch,

    #[error("unsupported cipher: {0}")]
    UnsupportedCipher(String),

    #[error("unsupported KDF: {0}")]
    UnsupportedKdf(String),

    #[error("no valid keyslot found")]
    NoKeyslot,

    #[error("passphrase does not match any keyslot")]
    WrongPassphrase,

    #[error("digest verification failed")]
    DigestMismatch,

    #[error("device-mapper operation failed: {0}")]
    DeviceMapper(String),

    #[error("JSON metadata error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("argon2 error: {0}")]
    Argon2(#[from] argon2::Error),

    #[error("invalid base64: {0}")]
    Base64(#[from] base64ct::Error),

    #[error("invalid header field: {0}")]
    InvalidField(String),

    #[error("TPM2 token not found")]
    NoTpm2Token,

    #[error("RNG failure")]
    Rng,
}

/// Result type for LUKS2 operations.
pub type Result<T> = core::result::Result<T, Luks2Error>;
