//! IPv4 and UDP header construction and parsing.

#![warn(missing_docs)]

pub mod checksum;
pub mod v4;

use core::result::Result as CoreResult;

use thiserror::Error;

/// Framing failures.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum Failure {
    /// Buffer is shorter than the structure it must hold.
    #[error("truncated buffer: {0} bytes available, {1} bytes required")]
    Truncated(usize, usize),
    /// IPv4 IHL field declares a header that does not fit the buffer.
    #[error("invalid IPv4 header length {0}")]
    BadIhl(usize),
    /// IPv4 protocol field is not UDP.
    #[error("non-UDP protocol {0}")]
    NotUdp(u8),
    /// A length does not fit the narrowest length field that must carry it.
    #[error("length {0} overflows its length field")]
    LengthOverflow(usize),
    /// Caller supplied buffer cannot hold the requested frame.
    #[error("buffer too small: {0} bytes required")]
    TooSmall(usize),
}

/// Framing result type.
pub type Result<T> = CoreResult<T, Failure>;
