//! Error types for the ramune initramfs builder.

use thiserror::Error;
use tokio::task::JoinError;

/// Error type for initramfs build operations.
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum RamuneError {
    #[error("Failed to read {file}: {source}")]
    ReadError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to write {file}: {source}")]
    WriteError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to initialize zstd encoder: {0}")]
    ZstdInitError(#[source] std::io::Error),

    #[error("Failed to finish zstd compression: {0}")]
    CompressionError(#[source] std::io::Error),

    #[error("Invalid compression level {level}; expected 0 or {min}..={max}")]
    InvalidCompressionLevel { level: i32, min: i32, max: i32 },

    #[error("Worker task failed: {0}")]
    TaskError(#[source] JoinError),

    #[error("Failed to create EROFS image: {0}")]
    ErofsError(String),

    #[error("Failed to create CPIO archive: {0}")]
    CpioError(String),
}

/// Result type alias for ramune operations.
pub type Result<T> = core::result::Result<T, RamuneError>;
