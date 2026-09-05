//! Error types for the koci library.

use thiserror::Error;

/// Error type for OCI image pulling and signing operations.
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum KociError {
    /// Failed to download an OCI image.
    #[error("Failed to download image: {0}")]
    DownloadError(String),

    /// Registry rejected the authentication attempt.
    #[error("Registry authentication failed for {registry}: {details}")]
    AuthError {
        /// Registry host the authentication failed against.
        registry: String,
        /// Failure details from the registry or the auth challenge.
        details: String,
    },

    /// OCI manifest or config is malformed.
    #[error("Invalid OCI format: {0}")]
    InvalidOciFormat(String),

    /// Failed to parse an OCI descriptor or manifest.
    #[error("OCI parsing error: {0}")]
    OciParseError(String),

    /// Failed to extract an OCI layer blob.
    #[error("Failed to extract layer: {0}")]
    LayerExtractionError(String),

    /// Layer media type is not supported.
    #[error("Unsupported OCI layer media type: {0}")]
    UnsupportedLayerMediaType(String),

    /// A network request failed.
    #[error("Network error: {0}")]
    NetworkError(String),

    /// An I/O error occurred.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

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

    /// Cryptographic signature verification failed.
    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),
}

/// Result type alias for koci operations.
pub type Result<T> = core::result::Result<T, KociError>;
