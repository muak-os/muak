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

    /// Request parameter validation failed.
    #[error("request validation: {0}")]
    RequestValidation(String),

    /// OCI source resolution failed.
    #[error("source resolution: {0}")]
    SourceResolution(String),

    /// Catalog document schema failure.
    #[error(transparent)]
    Catalog(#[from] kata::error::DocumentError),

    /// I/O error during build.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Disk plan or layout operation failed.
    #[error("disk: {0}")]
    Disk(#[from] disk::error::DiskError),

    /// Build process failed.
    #[error("build failure: {0}")]
    BuildError(String),
}

impl WizardError {
    /// Builds a pull failure naming the image reference and the cause.
    pub(crate) fn pull(context: &str, reference: &str, cause: impl core::fmt::Display) -> Self {
        Self::BuildError(format!("{context} {reference}: {cause}"))
    }
}

/// Result type alias for wizard operations.
pub type Result<T, E = WizardError> = core::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::WizardError;

    #[test]
    fn pull_names_the_context_and_the_reference() {
        // ARRANGE
        let cause = "Failed to pull image: blob download timed out";

        // ACT
        let message = WizardError::pull("pull kernel files", "ghcr.io/muak-os/linux:v1.0.0", cause)
            .to_string();

        // ASSERT
        assert_eq!(
            message,
            "build failure: pull kernel files ghcr.io/muak-os/linux:v1.0.0: \
             Failed to pull image: blob download timed out"
        );
    }

    #[test]
    fn pull_keeps_the_underlying_cause() {
        // ARRANGE
        let cause = "manifest digest mismatch";

        // ACT
        let message =
            WizardError::pull("open overlay session", "ghcr.io/muak-os/overlay", cause).to_string();

        // ASSERT
        assert!(
            message.contains("manifest digest mismatch"),
            "cause text must survive: {message}"
        );
        assert!(
            message.contains("ghcr.io/muak-os/overlay"),
            "reference must survive: {message}"
        );
    }
}
