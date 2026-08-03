//! Error types for the mumi image builder.

use thiserror::Error;

/// Error type for mumi image build operations.
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum MumiError {
    /// I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// EROFS image creation failed.
    #[error("Failed to create EROFS image: {0}")]
    Erofs(String),

    /// A caller-supplied argument is invalid.
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

/// Result type alias for mumi operations.
pub type Result<T> = core::result::Result<T, MumiError>;
