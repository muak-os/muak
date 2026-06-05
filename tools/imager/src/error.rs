//! Error types for the imager build pipeline.

use thiserror::Error;

/// Error type for imager operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum ImagerError {
    /// Profile validation failed.
    #[error("profile validation: {0}")]
    ProfileValidation(String),

    /// OCI source resolution failed.
    #[error("source resolution: {0}")]
    SourceResolution(String),

    /// Installer image is missing a required file.
    #[error("installer is missing required file: {0}")]
    MissingInstallerFile(String),

    /// Build process failed.
    #[error("build failure: {0}")]
    BuildError(String),
}

/// Result type alias for imager operations.
pub type Result<T, E = ImagerError> = core::result::Result<T, E>;
