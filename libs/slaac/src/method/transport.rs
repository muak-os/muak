//! Raw `ICMPv6` datagram transport the method drives.

use core::future::Future;
use core::net::Ipv6Addr;

/// Raw `ICMPv6` datagram transport.
pub trait Transport: Sync {
    /// Receives a payload and its optional source address.
    fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> impl Future<Output = std::io::Result<(usize, Option<Ipv6Addr>)>> + Send;

    /// Sends a payload to the given destination.
    fn send_to(
        &self,
        dst: Ipv6Addr,
        payload: &[u8],
    ) -> impl Future<Output = std::io::Result<usize>> + Send;
}
