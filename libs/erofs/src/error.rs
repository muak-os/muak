//! Error types for the erofs library.

use thiserror::Error;

/// Errors that can occur during EROFS image creation.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type intentionally matches the crate name"
)]
#[derive(Error, Debug)]
pub enum ErofsError {
    /// IO operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Source directory does not exist or is not a directory.
    #[error("invalid source directory: {0}")]
    InvalidSource(std::path::PathBuf),

    /// File too large for the format.
    #[error("file too large: {path}, size {size}")]
    FileTooLarge {
        /// Path of the oversized file.
        path: std::path::PathBuf,
        /// File size in bytes.
        size: u64,
    },

    /// Filename exceeds the 255-byte EROFS limit.
    #[error("filename too long: {0}")]
    FilenameTooLong(String),

    /// Symlink target read failed.
    #[error("failed to read symlink target: {0}")]
    SymlinkRead(std::path::PathBuf),

    /// Directory walk failed.
    #[error("directory walk error: {0}")]
    Walk(String),

    /// `file_contexts` parse error.
    #[error("file_contexts error: {0}")]
    FileContexts(String),

    /// Compression failed.
    #[error("compression error: {detail}")]
    Compression {
        /// Human-readable compression failure detail.
        detail: String,
    },

    /// Compression level is outside the zstd-supported range.
    #[error("invalid compression level {level}; expected 0 or {min}..={max}")]
    InvalidCompressionLevel {
        /// Invalid level provided by the caller.
        level: i32,
        /// Minimum supported compression level.
        min: i32,
        /// Maximum supported compression level.
        max: i32,
    },

    /// Internal invariant violated.
    #[error("internal error: {0}")]
    Internal(&'static str),
}

/// Result type alias for erofs operations.
pub type Result<T> = core::result::Result<T, ErofsError>;
