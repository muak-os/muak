//! Error types for the OCI data model.

use thiserror::Error;

/// Error type for pure OCI model operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum OciError {
    /// OCI manifest or config is malformed.
    #[error("Invalid OCI format: {0}")]
    InvalidFormat(String),

    /// Failed to parse an OCI descriptor or manifest.
    #[error("OCI parsing error: {0}")]
    Parse(String),

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Content digest does not match the expected value.
    #[error("Digest mismatch for {resource}: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Name of the resource with the mismatch.
        resource: String,
        /// Expected digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },
}

/// Result type alias for OCI model operations.
pub type Result<T> = core::result::Result<T, OciError>;
