//! Error types for ESP operations.

use fatfs::error::FatError;
use thiserror::Error;

/// Errors produced while building or populating an ESP.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "EspError is the canonical error type"
)]
pub enum EspError {
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The ESP path is invalid or inaccessible.
    #[error("Invalid ESP path: {0}")]
    InvalidPath(String),

    /// A source entry is not a supported type.
    #[error("Unsupported source entry: {0}")]
    UnsupportedEntry(String),

    /// FAT filesystem construction or writing failed.
    #[error("FAT filesystem error: {0}")]
    Fat(#[from] FatError),
}

/// Result type alias for ESP operations.
pub type Result<T> = core::result::Result<T, EspError>;
