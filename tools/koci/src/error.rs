//! Error types for the koci library.

use oci::error::OciError;
use oci_client::error::ClientError;
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
