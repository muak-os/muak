//! Reception and validation of `DHCPv4` responses on raw and unicast sockets.

use core::net::Ipv4Addr;
use core::time::Duration;

use dhcp_proto::{DHCP_CLIENT_PORT, ParsedOptions, header, message_type, validate};
use ip::v4::udp;
use packet::raw::Socket;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::{Failure, Result};

// Seconds to wait for a server response.
const TIMEOUT_SECS: u64 = 10;

pub(super) async fn raw(
    socket: &Socket,
    frame: &mut [u8],
    xid: u32,
    expected: u8,
) -> Result<(Ipv4Addr, ParsedOptions)> {
    loop {
        let read = timeout(Duration::from_secs(TIMEOUT_SECS), socket.recv(frame))
            .await
            .map_err(|_elapsed| Failure::Timeout)??;
        let Some(received) = frame.get(..read) else {
            continue;
        };
        let Ok((_src, dst, payload)) = udp::unwrap(received) else {
            continue;
        };
        if dst.port != DHCP_CLIENT_PORT {
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
) -> Result<(Ipv4Addr, ParsedOptions)> {
    let read = timeout(Duration::from_secs(TIMEOUT_SECS), socket.recv(frame))
        .await
        .map_err(|_elapsed| Failure::Timeout)??;
    let received = frame
        .get(..read)
        .ok_or(dhcp_proto::Failure::ResponseTooShort(read))?;
    let options = validate::response(received, xid, message_type::ACK)?;

    Ok((header::yiaddr(received), options))
}
