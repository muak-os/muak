//! Error type for firmware operations.

use thiserror::Error;

/// Error type for firmware operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "FirmwareError is the canonical error type"
)]
pub enum FirmwareError {
    /// Firmware variable storage operation failed.
    #[error("firmware variable: {0}")]
    Variable(String),

    /// I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for firmware operations.
pub type Result<T> = core::result::Result<T, FirmwareError>;
