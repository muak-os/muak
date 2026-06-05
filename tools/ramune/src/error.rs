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
    /// Failed to read a required file.
    #[error("Failed to read {file}: {source}")]
    ReadError {
        /// Path to the file that could not be read.
        file: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to write an output file.
    #[error("Failed to write {file}: {source}")]
    WriteError {
        /// Path to the file that could not be written.
        file: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to initialize the zstd encoder.
    #[error("Failed to initialize zstd encoder: {0}")]
    ZstdInitError(#[source] std::io::Error),

    /// Failed to finalize zstd compression.
    #[error("Failed to finish zstd compression: {0}")]
    CompressionError(#[source] std::io::Error),

    /// Compression level is outside the valid range.
    #[error("Invalid compression level {level}; expected 0 or {min}..={max}")]
    InvalidCompressionLevel {
        /// The invalid level provided.
        level: i32,
        /// Minimum valid level.
        min: i32,
        /// Maximum valid level.
        max: i32,
    },

    /// An async worker task failed.
    #[error("Worker task failed: {0}")]
    TaskError(#[source] JoinError),

    /// EROFS filesystem creation failed.
    #[error("Failed to create EROFS image: {0}")]
    ErofsError(String),

    /// CPIO archive creation failed.
    #[error("Failed to create CPIO archive: {0}")]
    CpioError(String),
}

/// Result type alias for ramune operations.
pub type Result<T> = core::result::Result<T, RamuneError>;
