//! Error types for network model operations.

use core::num::NonZeroI32;

use thiserror::Error;

/// Error returned by interface name validation.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
#[error("invalid interface name: {0:?}")]
pub struct NameError(pub(crate) String);

/// Errors reported by link monitoring.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LinkError {
    /// Link event buffer overran, so link events may have been lost.
    #[error("event buffer overran, events may be lost")]
    Overrun,
    /// Link monitor reported an error code.
    #[error("error code {0}")]
    Errno(NonZeroI32),
    /// Link event stream closed, so link events are no longer delivered.
    #[error("event stream closed")]
    Closed,
}
