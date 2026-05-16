//! Error types for ESP operations.

use thiserror::Error;

/// Errors produced while building or populating an ESP.
#[derive(Debug, Error)]
pub enum EspError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid ESP path: {0}")]
    InvalidPath(String),

    #[error("Unsupported source entry: {0}")]
    UnsupportedEntry(String),

    #[error("FAT filesystem error: {0}")]
    Fat(String),
}
