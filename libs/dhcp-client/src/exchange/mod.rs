//! `DHCPv4` exchanges: lease acquisition, renewal and rebinding (RFC 2131).

mod recv;

use core::net::Ipv4Addr;
use std::time::SystemTime;

use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT, builder, message_type, xid};
use ip::v4::{Addr, udp};
use packet::raw::{ETH_BROADCAST, Socket};
use tokio::net::UdpSocket;

use crate::lease::Lease;
use crate::{Failure, Result, Scratch};

/// Runs a full `DHCPDISCOVER` / `DHCPOFFER` / `DHCPREQUEST` / `DHCPACK` exchange.
///
/// # Errors
///
/// Returns [`Failure::Timeout`] when a response does not arrive in time, [`Failure::Nak`] when
/// the server refuses the request, [`Failure::MissingServerId`] when the offer carries no server
/// identifier, [`Failure::MissingAddress`] when a response carries no usable address, and any
/// message, framing or socket failure of the exchange.
pub async fn acquire(socket: &Socket, mac: [u8; 6], scratch: Scratch<'_>) -> Result<Lease> {
    let Scratch { msg, frame } = scratch;
    let xid = xid::generate()?;

    let discover = builder::discover(msg, xid, mac)?;
    send(
        socket,
        discover,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        frame,
    )
    .await?;

    let (offered, offer) = recv::raw(socket, frame, xid, message_type::OFFER).await?;
    let offered_ip = lease_address(offered, None)?;
    let server_id = offer.server_id.ok_or(Failure::MissingServerId)?;

    let request = builder::request(
        msg,
        xid,
        mac,
        offered_ip,
        builder::Mode::Selecting { server: server_id },
    )?;
    send(
        socket,
        request,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        frame,
    )
    .await?;

    let (received, ack) = recv::raw(socket, frame, xid, message_type::ACK).await?;
    let assigned_ip = lease_address(received, Some(offered_ip))?;

    Ok(Lease::from_ack(
        SystemTime::now(),
        assigned_ip,
        server_id,
        ack,
    ))
}

/// Sends a unicast `DHCPREQUEST` to the server that granted the lease.
///
/// # Errors
///
/// Returns [`Failure::Timeout`] when the ACK does not arrive in time, [`Failure::Nak`] when the
/// server refuses the renewal, and any message, framing or socket failure of the exchange.
pub async fn renew(
    socket: &UdpSocket,
    mac: [u8; 6],
    server: Ipv4Addr,
    assigned: Ipv4Addr,
    scratch: Scratch<'_>,
) -> Result<Lease> {
    let Scratch { msg, frame } = scratch;
    let xid = xid::generate()?;

    let request = builder::request(msg, xid, mac, assigned, builder::Mode::Unicast)?;
    socket.send_to(request, (server, DHCP_SERVER_PORT)).await?;

    let (received, ack) = recv::ack(socket, frame, xid, server).await?;
    let assigned_ip = lease_address(received, Some(assigned))?;

    Ok(Lease::from_ack(SystemTime::now(), assigned_ip, server, ack))
}

/// Sends a broadcast `DHCPREQUEST` from the assigned address to any server that answers.
///
/// # Errors
///
/// Returns [`Failure::Timeout`] when the ACK does not arrive in time, [`Failure::Nak`] when the
/// server refuses the rebind, and any message, framing or socket failure of the exchange.
pub async fn rebind(
    socket: &Socket,
    mac: [u8; 6],
    server: Ipv4Addr,
    assigned: Ipv4Addr,
    scratch: Scratch<'_>,
) -> Result<Lease> {
    let Scratch { msg, frame } = scratch;
    let xid = xid::generate()?;

    let request = builder::request(msg, xid, mac, assigned, builder::Mode::Broadcast)?;
    send(socket, request, assigned, Ipv4Addr::BROADCAST, frame).await?;

    let (received, ack) = recv::raw(socket, frame, xid, message_type::ACK).await?;
    let assigned_ip = lease_address(received, Some(assigned))?;

    Ok(Lease::from_ack(SystemTime::now(), assigned_ip, server, ack))
}

fn lease_address(received: Ipv4Addr, fallback: Option<Ipv4Addr>) -> Result<Ipv4Addr> {
    if !received.is_unspecified() {
        return Ok(received);
    }

    fallback.ok_or(Failure::MissingAddress)
}

async fn send(
    socket: &Socket,
    payload: &[u8],
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    frame: &mut [u8],
) -> Result<()> {
    let src = Addr {
        ip: src_ip,
        port: DHCP_CLIENT_PORT,
    };
    let dst = Addr {
        ip: dst_ip,
        port: DHCP_SERVER_PORT,
    };
    let wrapped = udp::wrap(payload, src, dst, frame)?;
    socket.send_to(wrapped, ETH_BROADCAST).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::net::Ipv4Addr;

    use super::lease_address;
    use crate::Failure;

    #[test]
    fn lease_address_prefers_received() {
        // ARRANGE
        let received = Ipv4Addr::new(10, 0, 0, 5);
        let fallback = Some(Ipv4Addr::new(10, 0, 0, 9));

        // ACT
        let result = lease_address(received, fallback);

        // ASSERT
        assert!(matches!(result, Ok(address) if address == received));
    }

    #[test]
    fn lease_address_uses_fallback_for_zero() {
        // ARRANGE
        let fallback = Some(Ipv4Addr::new(10, 0, 0, 9));

        // ACT
        let result = lease_address(Ipv4Addr::UNSPECIFIED, fallback);

        // ASSERT
        assert!(matches!(result, Ok(address) if address == Ipv4Addr::new(10, 0, 0, 9)));
    }

    #[test]
    fn lease_address_fails_without_fallback() {
        // ARRANGE
        let received = Ipv4Addr::UNSPECIFIED;

        // ACT
        let result = lease_address(received, None);

        // ASSERT
        assert!(matches!(result, Err(Failure::MissingAddress)));
    }
}
