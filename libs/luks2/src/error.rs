//! LUKS2 error types.

use thiserror::Error;

/// LUKS2 operation error types.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type intentionally matches the crate name"
)]
#[derive(Debug, Error)]
pub enum Luks2Error {
    /// LUKS2 magic bytes are invalid.
    #[error("invalid LUKS2 magic bytes")]
    InvalidMagic,

    /// LUKS version is not supported.
    #[error("unsupported LUKS version: {0}")]
    UnsupportedVersion(u16),

    /// Header checksum does not match.
    #[error("header checksum mismatch")]
    ChecksumMismatch,

    /// Cipher is not supported.
    #[error("unsupported cipher: {0}")]
    UnsupportedCipher(String),

    /// Key derivation function is not supported.
    #[error("unsupported KDF: {0}")]
    UnsupportedKdf(String),

    /// No valid keyslot was found.
    #[error("no valid keyslot found")]
    NoKeyslot,

    /// Passphrase does not match any keyslot.
    #[error("passphrase does not match any keyslot")]
    WrongPassphrase,

    /// Digest verification failed.
    #[error("digest verification failed")]
    DigestMismatch,

    /// Key length does not match the volume key size.
    #[error("invalid LUKS key length: {actual} bytes, expected {expected}")]
    InvalidKeyLength {
        /// Observed key length in bytes.
        actual: usize,
        /// Required key length in bytes.
        expected: usize,
    },

    /// Device-mapper operation failed.
    #[error("device-mapper operation failed: {0}")]
    DeviceMapper(String),

    /// JSON metadata is invalid.
    #[error("JSON metadata error: {0}")]
    Json(#[from] serde_json::Error),

    /// Underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps block device inspection failures.
    #[error("block device error: {0}")]
    Blockdev(#[from] blockdev::error::BlockdevError),

    /// Argon2 key derivation failed.
    #[error("argon2 error: {0}")]
    Argon2(#[from] argon2::Error),

    /// Base64 encoding or decoding failed.
    #[error("invalid base64: {0}")]
    Base64(#[from] base64ct::Error),

    /// Header field is invalid.
    #[error("invalid header field: {0}")]
    InvalidField(String),

    /// Random number generation failed.
    #[error("RNG failure")]
    Rng,
}

/// Result type for LUKS2 operations.
pub type Result<T> = core::result::Result<T, Luks2Error>;
