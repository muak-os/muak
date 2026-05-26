/// TPM2 error types.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Tpm2Error {
    #[error("TPM2 device not found")]
    DeviceNotFound,

    #[error("TPM2 I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TPM2 command failed: RC=0x{0:08X}")]
    TpmError(u32),

    #[error("TPM2 response too short: expected {expected}, got {actual}")]
    ResponseTooShort { expected: usize, actual: usize },

    #[error("TPM2 response tag mismatch")]
    BadResponseTag,

    #[error("invalid sealed blob format")]
    InvalidBlob,

    #[error("random number generation failed")]
    RngFailed,

    #[error("TPM2 buffer too large: {actual} bytes exceeds {max} bytes")]
    BufferTooLarge { actual: usize, max: usize },
}

pub type Result<T> = core::result::Result<T, Tpm2Error>;
