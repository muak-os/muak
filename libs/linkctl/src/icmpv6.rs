//! `ICMPv6` transport adapter over the raw packet socket.

use core::net::Ipv6Addr;

use slaac::method::transport::Transport;

/// Raw `ICMPv6` datagram socket the stateless method drives.
pub struct Socket(packet::icmpv6::Socket);

impl Socket {
    /// Opens a raw socket bound to `interface` and filtered to router advertisements.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying raw socket cannot be opened.
    pub fn open(interface: &str) -> std::io::Result<Self> {
        packet::icmpv6::Socket::open(interface, &[slaac::icmpv6::ROUTER_ADVERTISEMENT])
            .map(Self)
            .map_err(std::io::Error::other)
    }
}

impl Transport for Socket {
    async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, Option<Ipv6Addr>)> {
        packet::icmpv6::Socket::recv_from(&self.0, buf)
            .await
            .map_err(std::io::Error::other)
    }

    async fn send_to(&self, dst: Ipv6Addr, payload: &[u8]) -> std::io::Result<usize> {
        packet::icmpv6::Socket::send_to(&self.0, dst, payload)
            .await
            .map_err(std::io::Error::other)
    }
}
