//! `DHCPv4` client implementation for acquiring and renewing IPv4 leases.

use core::future::Future;
use core::net::Ipv4Addr;
use core::time::Duration;

use anyhow::Result;
use netlib::packet::{ETH_BROADCAST, Socket};
use netlib::socket::bind_device;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::Lease;
use super::codec::{ParsedOptions, build_lease_from_ack, generate_xid, validate_response};
use super::framing::{L3L4_HEADER_LEN, unwrap_ipv4_udp, wrap_ipv4_udp};
use super::packet::{
    DHCP_CLIENT_PORT, DHCP_SERVER_PORT, DHCP_TIMEOUT_SECS, append_param_request_list, build_header,
    build_request_message, message_type, option, yiaddr,
};

/// Abstracts DHCP socket creation for raw (broadcast) and unicast paths.
pub trait DhcpConnector: Clone + Send + Sync + 'static {
    /// Creates a raw `AF_PACKET` socket for broadcast DORA / REBIND.
    fn create_raw(&self, interface: &str) -> impl Future<Output = Result<Socket>> + Send;

    /// Creates a unicast UDP socket for RENEW.
    fn create_unicast(
        &self,
        interface: &str,
        src_ip: Ipv4Addr,
    ) -> impl Future<Output = Result<UdpSocket>> + Send;
}

/// System connector that opens raw and unicast sockets via the kernel.
#[derive(Clone, Default)]
pub struct SystemDhcpConnector;

impl DhcpConnector for SystemDhcpConnector {
    fn create_raw(&self, interface: &str) -> impl Future<Output = Result<Socket>> + Send {
        std::future::ready(Socket::open(interface).map_err(Into::into))
    }

    async fn create_unicast(&self, interface: &str, src_ip: Ipv4Addr) -> Result<UdpSocket> {
        let socket = UdpSocket::bind((src_ip, DHCP_CLIENT_PORT)).await?;
        bind_device(&socket, interface)?;

        Ok(socket)
    }
}

/// Runs a full DHCPDISCOVER->OFFER->REQUEST->ACK exchange via a raw packet socket.
///
/// # Errors
///
/// Returns an error if any step of the DORA exchange fails, including timeouts,
/// malformed packets, or a NAK from the server.
pub async fn run(socket: &Socket, mac: &[u8; 6]) -> Result<Lease> {
    let xid = generate_xid()?;

    let mut discover = build_header(xid, *mac);
    discover.reserve(3 + 6 + 1);
    discover.extend(&[option::MESSAGE_TYPE, 1, message_type::DISCOVER]);
    append_param_request_list(&mut discover);
    discover.push(option::END);

    println!("DHCP: sending DISCOVER xid={xid}");
    send_raw(
        socket,
        &discover,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
    )
    .await?;

    let (offer_buf, offer_opts, _) = recv_raw_validated(socket, xid, message_type::OFFER).await?;
    let offered_ip = yiaddr(&offer_buf);
    println!("DHCP: got OFFER yiaddr={offered_ip}");

    let server_id = offer_opts
        .server_id
        .ok_or_else(|| anyhow::anyhow!("no server identifier in DHCPOFFER"))?;

    let mut request = build_header(xid, *mac);
    request.reserve(3 + 6 + 6 + 6 + 1);
    request.extend(&[option::MESSAGE_TYPE, 1, message_type::REQUEST]);
    request.extend(&[option::REQUESTED_IP, 4]);
    request.extend(&offered_ip.octets());
    request.extend(&[option::SERVER_ID, 4]);
    request.extend(&server_id.octets());
    append_param_request_list(&mut request);
    request.push(option::END);

    println!("DHCP: sending REQUEST for {offered_ip}");
    send_raw(socket, &request, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST).await?;

    let (ack_buf, ack_opts, _) = recv_raw_validated(socket, xid, message_type::ACK).await?;
    let ip = yiaddr(&ack_buf);
    println!("DHCP: got ACK yiaddr={ip}");

    Ok(build_lease_from_ack(ip, server_id, &ack_opts))
}

/// Sends a unicast DHCP RENEW (REQUEST to the known server) per RFC 2131.
///
/// # Errors
///
/// Returns an error if the renewal exchange fails or the server NAKs.
pub async fn renew(
    socket: &UdpSocket,
    mac: &[u8; 6],
    server_ip: Ipv4Addr,
    assigned_ip: Ipv4Addr,
) -> Result<Lease> {
    println!("DHCP: sending RENEW to {server_ip} for {assigned_ip}");

    let xid = generate_xid()?;
    let msg = build_request_message(xid, *mac, assigned_ip, true);

    socket.send_to(&msg, (server_ip, DHCP_SERVER_PORT)).await?;

    receive_ack_unicast(socket, xid, server_ip).await
}

/// Sends a broadcast DHCP REBIND per RFC 2131 via raw socket.
///
/// # Errors
///
/// Returns an error if the rebind exchange fails or the server NAKs.
pub async fn rebind(
    socket: &Socket,
    mac: &[u8; 6],
    server_ip: Ipv4Addr,
    assigned_ip: Ipv4Addr,
) -> Result<Lease> {
    println!("DHCP: sending REBIND (broadcast) for {assigned_ip}");

    let xid = generate_xid()?;
    let msg = build_request_message(xid, *mac, assigned_ip, false);

    send_raw(socket, &msg, assigned_ip, Ipv4Addr::BROADCAST).await?;

    let (ack_buf, ack_opts, _) = recv_raw_validated(socket, xid, message_type::ACK).await?;
    let ip = yiaddr(&ack_buf);
    println!("DHCP: got ACK yiaddr={ip}");

    Ok(build_lease_from_ack(ip, server_ip, &ack_opts))
}

/// Receives one DHCP message of the expected type with matching xid via the raw socket.
async fn recv_raw_validated(
    socket: &Socket,
    xid_val: u32,
    expected_type: u8,
) -> Result<(Vec<u8>, ParsedOptions, usize)> {
    let mut buf = [0_u8; 2048];
    loop {
        let n = timeout(
            Duration::from_secs(DHCP_TIMEOUT_SECS),
            socket.recv(&mut buf),
        )
        .await??;
        if n < L3L4_HEADER_LEN {
            continue;
        }
        let Ok((payload, _src_port, dst_port)) = unwrap_ipv4_udp(buf.get(..n).unwrap_or_default())
        else {
            continue;
        };
        if dst_port != DHCP_CLIENT_PORT {
            continue;
        }
        match validate_response(payload, payload.len(), xid_val, expected_type) {
            Ok(opts) => return Ok((payload.to_vec(), opts, payload.len())),
            Err(e) if e.downcast_ref::<super::codec::DhcpNak>().is_some() => return Err(e),
            Err(_) => {}
        }
    }
}

/// Receives and validates a DHCP ACK on a unicast UDP socket.
async fn receive_ack_unicast(
    socket: &UdpSocket,
    xid_val: u32,
    server_ip: Ipv4Addr,
) -> Result<Lease> {
    let mut buf = [0_u8; 1500];
    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;
    let opts = validate_response(&buf, len, xid_val, message_type::ACK)?;
    let ip = yiaddr(&buf);
    println!("DHCP: got ACK yiaddr={ip}");

    Ok(build_lease_from_ack(ip, server_ip, &opts))
}

/// Sends a DHCP message wrapped in IPv4+UDP via the raw packet socket.
async fn send_raw(
    socket: &Socket,
    payload: &[u8],
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
) -> Result<()> {
    let frame = wrap_ipv4_udp(payload, src_ip, dst_ip, DHCP_CLIENT_PORT, DHCP_SERVER_PORT);
    socket.send_to(&frame, ETH_BROADCAST).await?;

    Ok(())
}
