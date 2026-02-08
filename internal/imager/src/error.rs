//! Error types and configuration for the imager library.

use thiserror::Error;

/// Error type for imager operations.
#[derive(Error, Debug)]
pub enum ImagerError {
    #[error("Failed to read {file}: {source}")]
    ReadError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to write {file}: {source}")]
    WriteError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to download image: {0}")]
    DownloadError(String),

    #[error("Invalid OCI format: {0}")]
    InvalidOciFormat(String),

    #[error("OCI parsing error: {0}")]
    OciParseError(String),

    #[error("Failed to extract layer: {0}")]
    LayerExtractionError(String),

    #[error("Failed to create squashfs: {0}")]
    SquashfsError(String),

    #[error("Failed to create cpio archive: {0}")]
    CpioError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Temporary directory error: {0}")]
    TempDirError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

/// Result type alias for imager operations.
pub type Result<T> = std::result::Result<T, ImagerError>;
