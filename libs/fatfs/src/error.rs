//! Error types for FAT filesystem operations.

use thiserror::Error;

/// Error type for FAT filesystem operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum FatError {
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// FAT filesystem construction or writing failed.
    #[error("FAT filesystem error: {0}")]
    Fat(String),
}

/// Result type alias for FAT filesystem operations.
pub type Result<T> = core::result::Result<T, FatError>;
