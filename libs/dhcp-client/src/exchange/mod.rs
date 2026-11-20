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
/// identifier, and any message, framing or socket failure of the exchange.
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

    let (offered_ip, offer) = recv::raw(socket, frame, xid, message_type::OFFER).await?;
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

    let (assigned_ip, ack) = recv::raw(socket, frame, xid, message_type::ACK).await?;

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

    let (assigned_ip, ack) = recv::ack(socket, frame, xid).await?;

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

    let (assigned_ip, ack) = recv::raw(socket, frame, xid, message_type::ACK).await?;

    Ok(Lease::from_ack(SystemTime::now(), assigned_ip, server, ack))
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
