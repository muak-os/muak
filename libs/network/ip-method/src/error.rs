//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by an address method.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum IpMethodError {
    /// Socket input or output failed.
    #[error("method I/O failed: {0}")]
    Io(#[source] io::Error),
    /// Protocol exchange failed.
    #[error("method protocol failed: {0}")]
    Protocol(String),
}

/// Shared result type for address method operations.
pub type Result<T> = core::result::Result<T, IpMethodError>;
