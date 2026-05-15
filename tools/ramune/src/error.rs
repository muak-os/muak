//! Error types for the ramune initramfs builder.

use thiserror::Error;

/// Error type for initramfs build operations.
#[derive(Error, Debug)]
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

    #[error("Extension worker task failed: {0}")]
    ExtensionTaskError(#[source] tokio::task::JoinError),

    #[error("Failed to create EROFS image: {0}")]
    ErofsError(String),

    #[error("Failed to create CPIO archive: {0}")]
    CpioError(String),
}

/// Result type alias for ramune operations.
pub type Result<T> = std::result::Result<T, RamuneError>;
