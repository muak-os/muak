//! Error construction shared by the image pull nodes.

use koci::error::KociError;

use crate::error::WizardError;

/// Wraps a pull failure into a build error naming the failed image reference.
pub(crate) fn pull_failure(context: &str, reference: &str, error: &KociError) -> WizardError {
    WizardError::BuildError(format!("{context} {reference}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_failure_names_the_reference_and_the_context() {
        // ARRANGE
        let error = KociError::Pull("blob download timed out".to_owned());

        // ACT
        let message =
            pull_failure("pull kernel files", "ghcr.io/muak-os/linux:v1.0.0", &error).to_string();

        // ASSERT
        assert_eq!(
            message,
            "build failure: pull kernel files ghcr.io/muak-os/linux:v1.0.0: \
             Failed to pull image: blob download timed out"
        );
    }

    #[test]
    fn pull_failure_keeps_the_underlying_cause() {
        // ARRANGE
        let error = KociError::Pull("manifest digest mismatch".to_owned());

        // ACT
        let message =
            pull_failure("open overlay session", "ghcr.io/muak-os/overlay", &error).to_string();

        // ASSERT
        assert!(
            message.contains("manifest digest mismatch"),
            "cause text must survive: {message}"
        );
    }
}
