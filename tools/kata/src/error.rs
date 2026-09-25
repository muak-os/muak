//! Error types for catalog schema and publisher operations.

use thiserror::Error;

/// Error type for parsing, validating, and serializing catalog documents.
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum DocumentError {
    /// A catalog document is malformed or violates the schema.
    #[error("Invalid catalog document: {0}")]
    Document(String),

    /// A catalog document failed to deserialize.
    #[error("Failed to parse catalog document: {0}")]
    Parse(#[from] toml::de::Error),

    /// A canonical document failed to serialize.
    #[error("Failed to serialize catalog document: {0}")]
    Serialize(String),
}

/// Result type alias for catalog schema operations.
pub type DocumentResult<T> = core::result::Result<T, DocumentError>;

/// Publisher-side error type.
#[cfg(feature = "cli")]
#[derive(Error, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
pub enum KataError {
    /// A catalog document is malformed or violates the schema.
    #[error("Invalid catalog document: {0}")]
    Document(String),

    /// A registry operation failed.
    #[error("Registry operation failed: {0}")]
    Registry(String),

    /// An entry is frozen within a release line and cannot change.
    #[error("Frozen catalog entry: {0}")]
    Frozen(String),

    /// An append-only publication gate prevented the operation.
    #[error("Append-only gate: {0}")]
    Gate(String),

    /// A lineage rule prevented composing from the requested line.
    #[error("Lineage check failed: {0}")]
    Lineage(String),

    /// A catalog schema operation failed.
    #[error(transparent)]
    Schema(#[from] DocumentError),

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for publisher operations.
#[cfg(feature = "cli")]
pub type Result<T> = core::result::Result<T, KataError>;
