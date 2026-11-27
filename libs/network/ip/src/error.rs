//! Error type and shared result alias.

use thiserror::Error;

/// Errors returned by IP framing operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum IpError {
    /// Buffer is shorter than the structure it must hold.
    #[error("truncated buffer: {0} bytes available, {1} bytes required")]
    Truncated(usize, usize),
    /// IPv4 IHL field declares a header that does not fit the buffer.
    #[cfg(feature = "v4")]
    #[error("invalid IPv4 header length {0}")]
    BadIhl(usize),
    /// IPv4 protocol field is not UDP.
    #[cfg(feature = "v4")]
    #[error("non-UDP protocol {0}")]
    NotUdp(u8),
    /// A length does not fit the narrowest length field that must carry it.
    #[error("length {0} overflows its length field")]
    LengthOverflow(usize),
    /// Caller supplied buffer cannot hold the requested frame.
    #[error("buffer too small: {0} bytes required")]
    TooSmall(usize),
}

/// Shared result type for IP framing operations.
pub type Result<T> = core::result::Result<T, IpError>;
