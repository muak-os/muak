//! Reception and validation of `DHCPv4` responses on raw and unicast sockets.

use core::net::Ipv4Addr;
use core::time::Duration;

use dhcp_proto::{DHCP_SERVER_PORT, ParsedOptions, header, message_type, validate};
use ip::v4::Addr;
use tokio::time::{Instant, timeout_at};

use crate::datagram::Datagram;
use crate::error::{DhcpError, Result};

// Seconds to wait for a server response.
const TIMEOUT_SECS: u64 = 10;

pub(super) async fn raw<D: Datagram>(
    socket: &D,
    frame: &mut [u8],
    xid: u32,
    expected: u8,
) -> Result<(Ipv4Addr, ParsedOptions)> {
    let deadline = deadline();
    loop {
        let Some(received) = timeout_at(deadline, socket.recv(frame))
            .await
            .map_err(|_elapsed| DhcpError::Timeout)??
        else {
            continue;
        };
        // Any server may answer a rebinding request, so the source address is not checked here,
        // only the source port.
        if received.src.port != DHCP_SERVER_PORT {
            continue;
        }
        match validate::response(received.payload, xid, expected) {
            Ok(options) => return Ok((header::yiaddr(received.payload), options)),
            Err(dhcp_proto::Failure::Nak) => return Err(DhcpError::Nak),
            Err(_other) => {}
        }
    }
}

pub(super) async fn ack<D: Datagram>(
    socket: &D,
    frame: &mut [u8],
    xid: u32,
    server: Ipv4Addr,
) -> Result<(Ipv4Addr, ParsedOptions)> {
    let deadline = deadline();
    loop {
        let Some(received) = timeout_at(deadline, socket.recv(frame))
            .await
            .map_err(|_elapsed| DhcpError::Timeout)??
        else {
            continue;
        };
        if !accepts(received.src, server) {
            continue;
        }
        match validate::response(received.payload, xid, message_type::ACK) {
            Ok(options) => return Ok((header::yiaddr(received.payload), options)),
            Err(dhcp_proto::Failure::Nak) => return Err(DhcpError::Nak),
            Err(_other) => {}
        }
    }
}

fn accepts(source: Addr, server: Ipv4Addr) -> bool {
    source.ip == server && source.port == DHCP_SERVER_PORT
}

fn deadline() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(TIMEOUT_SECS))
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod tests {
    use core::net::Ipv4Addr;

    use dhcp_proto::DHCP_SERVER_PORT;
    use ip::v4::Addr;

    use super::accepts;

    #[test]
    fn accepts_matching_address_and_port() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = Addr {
            ip: server,
            port: DHCP_SERVER_PORT,
        };

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(result);
    }

    #[test]
    fn rejects_wrong_address() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = Addr {
            ip: Ipv4Addr::new(10, 0, 0, 2),
            port: DHCP_SERVER_PORT,
        };

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn rejects_wrong_port() {
        // ARRANGE
        let server = Ipv4Addr::new(10, 0, 0, 1);
        let source = Addr {
            ip: server,
            port: DHCP_SERVER_PORT.saturating_add(1),
        };

        // ACT
        let result = accepts(source, server);

        // ASSERT
        assert!(!result);
    }
}
