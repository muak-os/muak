//! Error types for disk plan and layout operations.

use thiserror::Error;

use crate::plan::API_VERSION;

/// Error type for disk plan and layout operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "DiskError is the canonical error type"
)]
pub enum DiskError {
    /// The plan document uses a schema version this reader does not know.
    #[error("unsupported disk plan api_version '{0}' (supported: {API_VERSION})")]
    UnsupportedApiVersion(String),

    /// The plan document could not be parsed or serialized.
    #[error("invalid disk plan: {0}")]
    Toml(String),

    /// The document could not be converted to or from a plan.
    #[error("invalid plan conversion: {0}")]
    Plan(String),

    /// The document could not be read from or written to disk.
    #[error("disk plan io error")]
    Io(#[from] std::io::Error),

    /// A layout annotation named an unknown layout.
    #[error("unknown disk layout '{name}' (known: {known})")]
    UnknownLayout {
        /// The unknown layout name.
        name: String,
        /// Comma-separated known layout names.
        known: String,
    },
}
