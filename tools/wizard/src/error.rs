//! Error types for the wizard build pipeline.

use thiserror::Error;

/// Error type for wizard operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "Public error type intentionally includes 'wizard'"
)]
pub enum WizardError {
    /// Profile validation failed.
    #[error("profile validation: {0}")]
    ProfileValidation(String),

    /// OCI source resolution failed.
    #[error("source resolution: {0}")]
    SourceResolution(String),

    /// I/O error during build.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Build process failed.
    #[error("build failure: {0}")]
    BuildError(String),
}

/// Result type alias for wizard operations.
pub type Result<T, E = WizardError> = core::result::Result<T, E>;
