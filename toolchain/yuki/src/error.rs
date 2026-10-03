//! Error types for yuki operations.

use thiserror::Error;
use uki::error::UkiError;

/// Error type for custom errors in yuki operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum YukiError {
    /// Writing the output image failed.
    #[error("Failed to write output image: {0}")]
    Io(#[from] std::io::Error),

    /// PE structure is malformed or violates invariants.
    #[error("Invalid PE structure: {0}")]
    InvalidPeStructure(String),

    /// Too many sections to fit in the PE header.
    #[error("Too many sections: cannot add more sections to PE file")]
    TooManySections,

    /// Error from the underlying UKI library.
    #[error("UKI error: {0}")]
    Uki(#[from] UkiError),
}

/// Result type alias for yuki operations.
pub type Result<T> = core::result::Result<T, YukiError>;
