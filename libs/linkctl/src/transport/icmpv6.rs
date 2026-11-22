//! Raw `ICMPv6` datagram transport.

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

impl Transport for packet::icmpv6::Socket {
    async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, Option<Ipv6Addr>)> {
        packet::icmpv6::Socket::recv_from(self, buf)
            .await
            .map_err(std::io::Error::other)
    }

    async fn send_to(&self, dst: Ipv6Addr, payload: &[u8]) -> std::io::Result<usize> {
        packet::icmpv6::Socket::send_to(self, dst, payload)
            .await
            .map_err(std::io::Error::other)
    }
}
