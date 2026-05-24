//! Error types for yuki operations.

use thiserror::Error;

/// Error type for custom errors in yuki operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum YukiError {
    #[error("Failed to read {file}: {source}")]
    ReadError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to parse PE file: {0}")]
    PeParseError(String),

    #[error("Invalid PE structure: {0}")]
    InvalidPeStructure(String),

    #[error("Too many sections: cannot add more sections to PE file")]
    TooManySections,
}

/// Result type alias for yuki operations.
pub type Result<T> = core::result::Result<T, YukiError>;
