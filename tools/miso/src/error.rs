//! Error types for miso operations.

use esp::EspError;
use thiserror::Error;

/// Errors produced during image construction.
#[derive(Error, Debug)]
pub enum MisoError {
    /// Errors from file system and network I/O operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Errors from ESP-IDF ESP error type.
    #[error(transparent)]
    Esp(#[from] EspError),

    /// Errors from FAT filesystem operations.
    #[error("FAT filesystem error: {0}")]
    Fat(String),

    /// Errors from ISO 9660 structure validation and construction.
    #[error("ISO structure error: {0}")]
    Iso(String),

    /// Errors from GPT partition table operations.
    #[error("GPT error: {0}")]
    Gpt(String),

    /// Errors during zstd compression encoder initialization.
    #[error("Failed to initialize zstd encoder: {0}")]
    ZstdInit(#[source] std::io::Error),

    /// Errors when finalizing zstd compressed data.
    #[error("Failed to finish zstd compression: {0}")]
    Compression(#[source] std::io::Error),

    /// Invalid compression level provided to zstd encoder.
    #[error("Invalid compression level {level}; expected 0 or a value in [{min}, {max}]")]
    InvalidCompressionLevel {
        /// The invalid compression level that was provided.
        level: i32,
        /// Minimum allowed compression level.
        min: i32,
        /// Maximum allowed compression level.
        max: i32,
    },
}

impl From<parttable::error::ParttableError> for MisoError {
    fn from(err: parttable::error::ParttableError) -> Self {
        Self::Gpt(err.to_string())
    }
}
