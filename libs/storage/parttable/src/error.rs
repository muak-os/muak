//! Error types and shared result alias.

use thiserror::Error;

/// Errors returned by parttable operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum ParttableError {
    /// Wraps underlying I/O failures.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps underlying GPT encoding and decoding failures.
    #[error("GPT error: {0}")]
    Gpt(String),

    /// Reports invalid partition placement requests.
    #[error("Invalid partition placement: {0}")]
    InvalidPlacement(String),
}

/// Shared result type for parttable operations.
pub type Result<T> = core::result::Result<T, ParttableError>;
