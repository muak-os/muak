//! RFC 4861 Neighbor Discovery and RFC 4443 `ICMPv6` codec.

#![warn(missing_docs)]

pub mod build;
mod field;
pub mod message;
pub mod parse;
pub mod types;

use core::net::Ipv6Addr;
use core::result::Result as CoreResult;

use ip::v6;
use thiserror::Error;

/// IPv6 all-routers link-local multicast address.
pub const ALL_ROUTERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);

/// IPv6 all-nodes link-local multicast address.
pub const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

/// Upper-layer protocol number of `ICMPv6`.
pub const NEXT_HEADER_ICMPV6: u8 = 58;

/// `ICMPv6` Router Solicitation type code.
pub const ROUTER_SOLICITATION: u8 = 133;
/// `ICMPv6` Router Advertisement type code.
pub const ROUTER_ADVERTISEMENT: u8 = 134;
/// `ICMPv6` Neighbor Solicitation type code.
pub const NEIGHBOR_SOLICITATION: u8 = 135;
/// `ICMPv6` Neighbor Advertisement type code.
pub const NEIGHBOR_ADVERTISEMENT: u8 = 136;

/// `ICMPv6` codec failures.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum Failure {
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

/// `ICMPv6` codec result type.
pub type Result<T> = CoreResult<T, Failure>;

/// Computes the `ICMPv6` checksum of `message` over the RFC 8200 section 8.1 pseudo-header.
#[must_use = "callers need the checksum"]
pub fn checksum(source: Ipv6Addr, destination: Ipv6Addr, message: &[u8]) -> u16 {
    v6::sum(source, destination, NEXT_HEADER_ICMPV6, message)
}
