//! Error type and shared result alias.

use thiserror::Error;

/// Errors returned by `ICMPv6` codec operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum NdpError {
    /// Message or option is shorter than its declared or minimum length.
    #[error("truncated ICMPv6 data: {0} bytes available, {1} bytes required")]
    Truncated(usize, usize),
    /// Message type does not match the message that was requested.
    #[error("unexpected ICMPv6 type {0}")]
    WrongType(u8),
    /// Caller supplied buffer cannot hold the message.
    #[error("buffer too small: {0} bytes required")]
    TooSmall(usize),
    /// A length does not fit the field that must carry it.
    ///
    /// The value is the offending option length in 8 byte units, as RFC 4861 option length fields count 8 byte units.
    #[error("length {0} does not fit its length field")]
    LengthOverflow(usize),
}

/// Shared result type for `ICMPv6` codec operations.
pub type Result<T> = core::result::Result<T, NdpError>;
