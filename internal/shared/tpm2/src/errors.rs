/// TPM2 error types.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("TPM2 device not found at {0}")]
    DeviceNotFound(String),

    #[error("TPM2 I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TPM2 command failed: RC=0x{0:08X}")]
    TpmError(u32),

    #[error("TPM2 response too short: expected {expected}, got {actual}")]
    ResponseTooShort { expected: usize, actual: usize },

    #[error("TPM2 response tag mismatch")]
    BadResponseTag,

    #[error("TPM2 unseal failed")]
    UnsealFailed,

    #[error("TPM2 SRK not found and creation failed")]
    NoSrk,

    #[error("invalid sealed blob format")]
    InvalidBlob,
}

pub type Result<T> = std::result::Result<T, Error>;
