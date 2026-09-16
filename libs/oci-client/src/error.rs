//! Error types for the distribution-spec client.

use thiserror::Error;

/// Error type for registry transport and authentication operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum ClientError {
    /// Failed to download from the registry.
    #[error("Failed to download image: {0}")]
    Download(String),

    /// Failed to push to the registry.
    #[error("Failed to push image: {0}")]
    Push(String),

    /// Registry rejected the authentication attempt.
    #[error("Registry authentication failed for {registry}: {details}")]
    Auth {
        /// Registry host the authentication failed against.
        registry: String,
        /// Failure details from the registry or the auth challenge.
        details: String,
    },

    /// A network request failed.
    #[error("Network error: {0}")]
    Network(String),
}

/// Result type alias for distribution-spec client operations.
pub type Result<T> = core::result::Result<T, ClientError>;
