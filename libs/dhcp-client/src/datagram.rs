//! Transport seam for `DHCPv4` exchanges.

use core::future::Future;
use core::net::SocketAddr;

use dhcp_proto::DHCP_CLIENT_PORT;
use ip::v4::{Addr, udp};
use packet::v4::{ETH_BROADCAST, Socket};
use tokio::net::UdpSocket;

use crate::Result;

/// A datagram transport carrying one `DHCPv4` exchange.
pub trait Datagram: Send + Sync {
    /// Receives one datagram into `frame`, or `None` when it carries no usable message.
    fn recv<'a>(
        &'a self,
        frame: &'a mut [u8],
    ) -> impl Future<Output = Result<Option<Received<'a>>>> + Send + 'a;

    /// Sends `payload` from `src` to `dst`, framing it into `frame` when the transport is raw.
    fn send<'a>(
        &'a self,
        payload: &'a [u8],
        src: Addr,
        dst: Addr,
        frame: &'a mut [u8],
    ) -> impl Future<Output = Result<()>> + Send + 'a;
}

/// A received datagram with the UDP address that carried it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received<'a> {
    /// Sender address.
    pub src: Addr,
    /// UDP payload.
    pub payload: &'a [u8],
}

impl Datagram for Socket {
    async fn recv<'a>(&'a self, frame: &'a mut [u8]) -> Result<Option<Received<'a>>> {
        let read = Socket::recv(self, frame).await?;
        let Some(received) = frame.get(..read) else {
            return Ok(None);
        };

        Ok(raw_received(received))
    }

    async fn send<'a>(
        &'a self,
        payload: &'a [u8],
        src: Addr,
        dst: Addr,
        frame: &'a mut [u8],
    ) -> Result<()> {
        let wrapped = udp::wrap(payload, src, dst, frame)?;
        Socket::send_to(self, wrapped, ETH_BROADCAST).await?;

        Ok(())
    }
}

impl Datagram for UdpSocket {
    async fn recv<'a>(&'a self, frame: &'a mut [u8]) -> Result<Option<Received<'a>>> {
        let (read, source) = UdpSocket::recv_from(self, frame).await?;
        let Some(received) = frame.get(..read) else {
            return Ok(None);
        };

        Ok(unicast_received(source, received))
    }

    async fn send<'a>(
        &'a self,
        payload: &'a [u8],
        _src: Addr,
        dst: Addr,
        _frame: &'a mut [u8],
    ) -> Result<()> {
        UdpSocket::send_to(self, payload, (dst.ip, dst.port)).await?;

        Ok(())
    }
}

fn raw_received(buffer: &[u8]) -> Option<Received<'_>> {
    let (src, dst, payload) = udp::unwrap(buffer).ok()?;
    if dst.port != DHCP_CLIENT_PORT {
        return None;
    }

    Some(Received { src, payload })
}

const fn unicast_received(source: SocketAddr, buffer: &[u8]) -> Option<Received<'_>> {
    let SocketAddr::V4(source) = source else {
        return None;
    };

    Some(Received {
        src: Addr {
            ip: *source.ip(),
            port: source.port(),
        },
        payload: buffer,
    })
}

#[cfg(test)]
mod tests {
    use core::net::{Ipv4Addr, SocketAddr};

    use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT};
    use ip::v4::{Addr, udp};

    use super::{raw_received, unicast_received};

    fn endpoints() -> (Addr, Addr) {
        (
            Addr {
                ip: Ipv4Addr::new(10, 0, 0, 1),
                port: DHCP_SERVER_PORT,
            },
            Addr {
                ip: Ipv4Addr::BROADCAST,
                port: DHCP_CLIENT_PORT,
            },
        )
    }

    #[test]
    fn raw_received_parses_a_client_frame() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buffer = [0_u8; 64];
        let frame = udp::wrap(b"dhcp", src, dst, &mut buffer).expect("wrap writes a frame");

        // ACT
        let received = raw_received(frame).expect("the frame is addressed to the client port");

        // ASSERT
        assert_eq!(received.src, src);
        assert_eq!(received.payload, b"dhcp");
    }

    #[test]
    fn raw_received_ignores_other_ports() {
        // ARRANGE
        let (src, _dst) = endpoints();
        let dst = Addr {
            ip: Ipv4Addr::BROADCAST,
            port: DHCP_CLIENT_PORT.saturating_add(1),
        };
        let mut buffer = [0_u8; 64];
        let frame = udp::wrap(b"dhcp", src, dst, &mut buffer).expect("wrap writes a frame");

        // ACT
        let received = raw_received(frame);

        // ASSERT
        assert!(received.is_none());
    }

    #[test]
    fn raw_received_ignores_malformed_frames() {
        // ARRANGE
        let frame = [0_u8; 8];

        // ACT
        let received = raw_received(&frame);

        // ASSERT
        assert!(received.is_none());
    }

    #[test]
    fn unicast_received_keeps_the_source() {
        // ARRANGE
        let source = SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), DHCP_SERVER_PORT));

        // ACT
        let received = unicast_received(source, b"dhcp").expect("an IPv4 source is accepted");

        // ASSERT
        assert_eq!(
            received.src,
            Addr {
                ip: Ipv4Addr::new(10, 0, 0, 1),
                port: DHCP_SERVER_PORT,
            }
        );
        assert_eq!(received.payload, b"dhcp");
    }

    #[test]
    fn unicast_received_ignores_ipv6_sources() {
        // ARRANGE
        let source = SocketAddr::from((core::net::Ipv6Addr::LOCALHOST, DHCP_SERVER_PORT));

        // ACT
        let received = unicast_received(source, b"dhcp");

        // ASSERT
        assert!(received.is_none());
    }
}
