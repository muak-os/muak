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
    /// Netlink notification buffer overran, so link events may have been lost.
    #[error("netlink notification buffer overran, events may be lost")]
    Overrun,
    /// Netlink reported an error on the event socket.
    #[error("netlink event socket error: {0}")]
    Kernel(NonZeroI32),
    /// Netlink event stream closed, so link events are no longer delivered.
    #[error("netlink event stream closed")]
    Closed,
}
