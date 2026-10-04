//! Error types for the koci library.

use oci::error::OciError;
use oci_client::error::ClientError;
use oci_client::retry;
use thiserror::Error;

/// Error type for koci operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum KociError {
    /// Registry transport or authentication failure.
    #[error(transparent)]
    Client(#[from] ClientError),

    /// Pure OCI model failure.
    #[error(transparent)]
    Oci(#[from] OciError),

    /// Pull orchestration failure.
    #[error("Failed to pull image: {0}")]
    Pull(String),

    /// Push orchestration failure.
    #[error("Failed to push image: {0}")]
    PushError(String),

    /// Merge orchestration failure.
    #[error("Failed to merge index: {0}")]
    MergeError(String),

    /// Copy orchestration failure.
    #[error("Failed to copy image: {0}")]
    CopyError(String),

    /// Failed to extract a layer blob.
    #[error("Failed to extract layer: {0}")]
    LayerExtractionError(String),

    /// Layer media type is not supported.
    #[error("Unsupported OCI layer media type: {0}")]
    UnsupportedLayerMediaType(String),

    /// Cryptographic signature verification failed.
    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    /// An I/O error occurred.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON serialization or deserialization failed.
    #[cfg(feature = "json")]
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

/// Result type alias for koci operations.
pub type Result<T> = core::result::Result<T, KociError>;

impl KociError {
    /// Whether retrying the failed operation may still succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Client(error) if retry::is_retryable(error))
            || matches!(self, Self::IoError(error) if is_retryable_io(error.kind()))
    }
}

/// Whether an IO failure of this kind may succeed on a later attempt.
fn is_retryable_io(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_failures_inherit_the_transport_classification() {
        // ARRANGE
        let transient = KociError::from(ClientError::Network("timeout".to_owned()));
        let permanent = KociError::from(ClientError::Status {
            status: 404,
            url: "http://registry/v2/".to_owned(),
        });

        // ACT / ASSERT
        assert!(transient.is_retryable());
        assert!(!permanent.is_retryable());
    }

    #[test]
    fn io_failures_are_retryable_only_by_kind() {
        // ARRANGE
        let transient = KociError::IoError(std::io::Error::from(std::io::ErrorKind::TimedOut));
        let permanent = KociError::IoError(std::io::Error::from(std::io::ErrorKind::InvalidData));

        // ACT / ASSERT
        assert!(transient.is_retryable());
        assert!(!permanent.is_retryable());
    }

    #[test]
    fn orchestration_failures_are_never_retryable() {
        // ARRANGE
        let errors = [
            KociError::Pull("interrupted stream".to_owned()),
            KociError::LayerExtractionError("bad entry".to_owned()),
        ];

        // ACT / ASSERT
        for error in &errors {
            assert!(!error.is_retryable(), "{error} must not retry");
        }
    }
}
