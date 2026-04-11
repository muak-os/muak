//! DHCPv4 client implementation for acquiring and renewing IPv4 leases.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::Result;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::DhcpLease;
use super::codec::{build_lease_from_ack, generate_xid, validate_response};
use super::packet::{
    DHCP_CLIENT_PORT, DHCP_SERVER_PORT, DHCP_TIMEOUT_SECS, append_param_request_list, build_header,
    build_request_message, message_type, option, yiaddr,
};

/// Receives and validates a DHCP ACK, building a `DhcpLease` from the response.
async fn receive_ack(socket: &UdpSocket, xid_val: u32, server_ip: Ipv4Addr) -> Result<DhcpLease> {
    let mut buf = [0u8; 1500];
    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;

    let ack_opts = validate_response(&buf, len, xid_val, message_type::ACK)?;
    let ip = yiaddr(&buf);
    println!("DHCP: got ACK yiaddr={}", ip);

    build_lease_from_ack(ip, server_ip, &ack_opts)
}

/// Runs a full DHCPDISCOVER->OFFER->REQUEST->ACK exchange on the given interface.
pub async fn run_dhcp_client(interface: &str, mac: &[u8; 6]) -> Result<DhcpLease> {
    println!("DHCP: starting on {}", interface);

    let socket = UdpSocket::bind(("0.0.0.0", DHCP_CLIENT_PORT)).await?;
    socket.set_broadcast(true)?;
    netlib::socket::bind_device(&socket, interface)?;

    let xid = generate_xid()?;

    let mut discover = build_header(xid, mac);
    discover.extend(&[option::MESSAGE_TYPE, 1, message_type::DISCOVER]);
    append_param_request_list(&mut discover);
    discover.push(option::END);

    println!("DHCP: sending DISCOVER xid={}", xid);
    socket
        .send_to(&discover, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    let mut buf = [0u8; 1500];
    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;
    let offer_opts = validate_response(&buf, len, xid, message_type::OFFER)?;
    let offered_ip = yiaddr(&buf);
    println!("DHCP: got OFFER yiaddr={}", offered_ip);

    let server_id = offer_opts
        .server_id
        .ok_or_else(|| anyhow::anyhow!("no server identifier in DHCPOFFER"))?;

    let mut request = build_header(xid, mac);
    request.extend(&[option::MESSAGE_TYPE, 1, message_type::REQUEST]);
    request.extend(&[option::REQUESTED_IP, 4]);
    request.extend(&offered_ip.octets());
    request.extend(&[option::SERVER_ID, 4]);
    request.extend(&server_id.octets());
    append_param_request_list(&mut request);
    request.push(option::END);

    println!("DHCP: sending REQUEST for {}", offered_ip);
    socket
        .send_to(&request, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    receive_ack(&socket, xid, server_id).await
}

/// Sends a unicast DHCP RENEW (REQUEST to the known server) per RFC 2131.
pub async fn renew_dhcp_client(
    interface: &str,
    mac: &[u8; 6],
    server_ip: Ipv4Addr,
    assigned_ip: Ipv4Addr,
) -> Result<DhcpLease> {
    println!("DHCP: sending RENEW to {} for {}", server_ip, assigned_ip);

    let socket = UdpSocket::bind(("0.0.0.0", DHCP_CLIENT_PORT)).await?;
    netlib::socket::bind_device(&socket, interface)?;

    let xid = generate_xid()?;
    let msg = build_request_message(xid, mac, assigned_ip, true);

    socket.send_to(&msg, (server_ip, DHCP_SERVER_PORT)).await?;

    receive_ack(&socket, xid, server_ip).await
}

/// Sends a broadcast DHCP REBIND (REQUEST to any server) per RFC 2131.
pub async fn rebind_dhcp_client(
    interface: &str,
    mac: &[u8; 6],
    server_ip: Ipv4Addr,
    assigned_ip: Ipv4Addr,
) -> Result<DhcpLease> {
    println!("DHCP: sending REBIND (broadcast) for {}", assigned_ip);

    let socket = UdpSocket::bind(("0.0.0.0", DHCP_CLIENT_PORT)).await?;
    socket.set_broadcast(true)?;
    netlib::socket::bind_device(&socket, interface)?;

    let xid = generate_xid()?;
    let msg = build_request_message(xid, mac, assigned_ip, false);

    socket
        .send_to(&msg, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    receive_ack(&socket, xid, server_ip).await
}
