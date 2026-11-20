//! Reception and validation of `DHCPv4` responses on raw and unicast sockets.

use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

use dhcp_proto::{
    DHCP_CLIENT_PORT, DHCP_SERVER_PORT, ParsedOptions, header, message_type, validate,
};
use ip::v4::udp;
use packet::raw::Socket;
use tokio::net::UdpSocket;
use tokio::time::{Instant, timeout_at};

use crate::{Failure, Result};

// Seconds to wait for a server response.
const TIMEOUT_SECS: u64 = 10;

pub(super) async fn raw(
    socket: &Socket,
    frame: &mut [u8],
    xid: u32,
    expected: u8,
) -> Result<(Ipv4Addr, ParsedOptions)> {
    let deadline = deadline();
    loop {
        let read = timeout_at(deadline, socket.recv(frame))
            .await
            .map_err(|_elapsed| Failure::Timeout)??;
        let Some(received) = frame.get(..read) else {
            continue;
        };
        let Ok((src, dst, payload)) = udp::unwrap(received) else {
            continue;
        };
        // Any server may answer a rebinding request, so the source address is not checked here,
        // only the ports.
        if dst.port != DHCP_CLIENT_PORT || src.port != DHCP_SERVER_PORT {
            continue;
        }
        match validate::response(payload, xid, expected) {
            Ok(options) => return Ok((header::yiaddr(payload), options)),
            Err(dhcp_proto::Failure::Nak) => return Err(Failure::Nak),
            Err(_other) => {}
        }
    }
}

pub(super) async fn ack(
    socket: &UdpSocket,
    frame: &mut [u8],
    xid: u32,
    server: Ipv4Addr,
) -> Result<(Ipv4Addr, ParsedOptions)> {
    let deadline = deadline();
    loop {
        let (read, source) = timeout_at(deadline, socket.recv_from(frame))
            .await
            .map_err(|_elapsed| Failure::Timeout)??;
        if !accepts(source, server) {
            continue;
        }
        let Some(received) = frame.get(..read) else {
            continue;
        };
        match validate::response(received, xid, message_type::ACK) {
            Ok(options) => return Ok((header::yiaddr(received), options)),
            Err(dhcp_proto::Failure::Nak) => return Err(Failure::Nak),
            Err(_other) => {}
        }
    }
}

fn accepts(source: SocketAddr, server: Ipv4Addr) -> bool {
    match source {
        SocketAddr::V4(v4) => *v4.ip() == server && v4.port() == DHCP_SERVER_PORT,
        SocketAddr::V6(_) => false,
    }
}

fn deadline() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(TIMEOUT_SECS))
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod tests {
    use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

    use dhcp_proto::DHCP_SERVER_PORT;

    use super::accepts;

    #[test]
    fn accepts_matching_address_and_port() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = SocketAddr::from((server, DHCP_SERVER_PORT));

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(result);
    }

    #[test]
    fn rejects_wrong_address() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = SocketAddr::from((Ipv4Addr::new(10, 0, 0, 2), DHCP_SERVER_PORT));

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn rejects_wrong_port() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = SocketAddr::from((server, DHCP_SERVER_PORT.saturating_add(1)));

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn rejects_ipv6_source() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = SocketAddr::from((Ipv6Addr::LOCALHOST, DHCP_SERVER_PORT));

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(!result);
    }
}
