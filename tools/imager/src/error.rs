//! Error types for the imager build pipeline.

use thiserror::Error;

/// Error type for imager operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum ImagerError {
    #[error("profile validation: {0}")]
    ProfileValidation(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result type alias for imager operations.
pub type Result<T, E = ImagerError> = core::result::Result<T, E>;
