//! Error types for yuki operations.

use thiserror::Error;

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

    /// PE parsing failed with a system-level error.
    #[error("Failed to parse PE file: {0}")]
    PeParseError(String),

    /// PE structure is malformed or violates invariants.
    #[error("Invalid PE structure: {0}")]
    InvalidPeStructure(String),

    /// PE section count would exceed the maximum.
    #[error("Too many sections: cannot add more sections to PE file")]
    TooManySections,
}

/// Result type alias for yuki operations.
pub type Result<T> = core::result::Result<T, YukiError>;
