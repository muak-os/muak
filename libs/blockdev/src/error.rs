//! Error types and shared result alias.

use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by blockdev operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum BlockdevError {
    /// Wraps underlying I/O failures.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Reports a missing root directory, normally an unmounted sysfs.
    #[error(
        "{} does not exist, sysfs may not be mounted",
        .0.display()
    )]
    MissingRoot(PathBuf),
}

/// Shared result type for blockdev operations.
pub type Result<T> = core::result::Result<T, BlockdevError>;
