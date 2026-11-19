//! IPv4 datagram framing.

pub mod header;
pub mod udp;

use core::net::Ipv4Addr;
use core::ops::Range;

/// A UDP endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addr {
    /// Endpoint address.
    pub ip: Ipv4Addr,
    /// Endpoint port.
    pub port: u16,
}

/// Returns the sub-slice of `data` described by `range`, or `None` when it does not fit.
pub(crate) fn slot(data: &mut [u8], range: Range<usize>) -> Option<&mut [u8]> {
    data.get_mut(range)
}
