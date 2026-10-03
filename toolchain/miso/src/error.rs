//! Error types for miso operations.

use esp::error::EspError;
use parttable::error::ParttableError;
use thiserror::Error;

/// Errors produced during image construction.
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
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
}

impl From<ParttableError> for MisoError {
    fn from(err: ParttableError) -> Self {
        Self::Gpt(err.to_string())
    }
}

/// Result type alias for miso operations.
pub type Result<T> = core::result::Result<T, MisoError>;
