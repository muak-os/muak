use thiserror::Error;

/// Errors that can occur during UKI and PE parsing.
#[expect(
    clippy::module_name_repetitions,
    reason = "This is the primary error type for the uki crate"
)]
#[derive(Error, Debug)]
pub enum UkiError {
    /// The PE file is malformed or violates invariants.
    #[error("invalid PE: {0}")]
    InvalidPe(&'static str),

    /// An arithmetic overflow occurred.
    #[error("overflow: {0}")]
    Overflow(&'static str),

    /// An I/O error occurred while reading PE data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for `uki` operations.
pub type Result<T> = core::result::Result<T, UkiError>;
