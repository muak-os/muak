//! Error types for the btrfs library.

use std::io::Error as IoError;
use std::path::PathBuf;

use rustix::io::Errno;
use thiserror::Error;

/// Errors that can occur in btrfs operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum BtrfsError {
    /// IO operation failed.
    #[error("IO error: {0}")]
    Io(#[from] IoError),

    /// System call error.
    #[error("System error: {0}")]
    Errno(#[from] Errno),

    /// Subvolume operation failed.
    #[error("Failed to {operation} subvolume {path}: {source}")]
    Subvolume {
        operation: &'static str,
        path: PathBuf,
        source: Errno,
    },

    /// Quota enable failed.
    #[error("Failed to enable btrfs quota on {mount_point}: {source}")]
    QuotaEnable { mount_point: String, source: Errno },

    /// Quota limit failed.
    #[error("Failed to set quota limit on {path}: {source}")]
    QuotaLimit { path: PathBuf, source: Errno },

    /// Quota usage lookup failed.
    #[error("Failed to read quota usage for {path}: {source}")]
    QuotaLookup { path: PathBuf, source: Errno },

    /// Scrub operation failed.
    #[error("Failed to scrub {mount_point}: {source}")]
    Scrub { mount_point: String, source: Errno },

    /// Filesystem creation failed.
    #[error("Failed to create btrfs filesystem: {0}")]
    Mkfs(String),

    /// Device too small.
    #[error("Device too small for btrfs: need at least {min_size} bytes, have {actual_size}")]
    DeviceTooSmall { min_size: u64, actual_size: u64 },

    /// Invalid argument or state.
    #[error("{0}")]
    InvalidArgument(String),
}

/// Result type alias for btrfs operations.
pub type Result<T> = core::result::Result<T, BtrfsError>;
