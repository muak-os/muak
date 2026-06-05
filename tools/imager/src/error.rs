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

    #[error("source resolution: {0}")]
    SourceResolution(String),

    #[error("installer is missing required file: {0}")]
    MissingInstallerFile(String),

    #[error("build failure: {0}")]
    BuildError(String),
}

/// Result type alias for imager operations.
pub type Result<T, E = ImagerError> = core::result::Result<T, E>;
