//! `ICMPv6` framing shared by every Neighbor Discovery message.

use core::net::Ipv6Addr;

use ip::v6;

/// Offset of the `ICMPv6` checksum field.
pub(crate) const CHECKSUM: usize = 2;
/// Length of the `ICMPv6` checksum field.
pub(crate) const CHECKSUM_LEN: usize = 2;

/// Upper-layer protocol number of `ICMPv6`.
pub const NEXT_HEADER_ICMPV6: u8 = 58;

/// Computes the `ICMPv6` checksum of `message` over the RFC 8200 section 8.1 pseudo-header.
#[must_use = "callers need the checksum"]
pub fn checksum(source: Ipv6Addr, destination: Ipv6Addr, message: &[u8]) -> u16 {
    v6::sum(source, destination, NEXT_HEADER_ICMPV6, message)
}
