//! Error types for miso operations.

use thiserror::Error;

/// Errors produced by miso during FAT image or ISO construction.
#[derive(Error, Debug)]
pub enum MisoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("FAT filesystem error: {0}")]
    Fat(String),

    #[error("ISO structure error: {0}")]
    Iso(String),
}
