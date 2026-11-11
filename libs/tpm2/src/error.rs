//! TPM2 error types.

use thiserror::Error;

/// TPM2 operation error types.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type intentionally matches the crate name"
)]
#[derive(Debug, Error)]
pub enum Tpm2Error {
    /// TPM2 device is not available.
    #[error("TPM2 device not found")]
    DeviceNotFound,

    /// Underlying I/O operation failed.
    #[error("TPM2 I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// TPM2 command returned a failure response code.
    #[error("TPM2 command failed: RC=0x{0:08X}")]
    TpmError(u32),

    /// TPM2 response is shorter than required.
    #[error("TPM2 response too short: expected {expected}, got {actual}")]
    ResponseTooShort {
        /// Expected response length in bytes.
        expected: usize,
        /// Actual response length in bytes.
        actual: usize,
    },

    /// TPM2 response tag is not recognized.
    #[error("TPM2 response tag mismatch")]
    BadResponseTag,

    /// Sealed blob has an invalid format.
    #[error("invalid sealed blob format")]
    InvalidBlob,

    /// Random number generation failed.
    #[error("random number generation failed")]
    RngFailed,

    /// Command buffer exceeds the protocol limit.
    #[error("TPM2 buffer too large: {actual} bytes exceeds {max} bytes")]
    BufferTooLarge {
        /// Requested buffer length in bytes.
        actual: usize,
        /// Maximum supported buffer length in bytes.
        max: usize,
    },
}

/// Result type for TPM2 operations.
pub type Result<T> = core::result::Result<T, Tpm2Error>;
