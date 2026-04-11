//! DHCPv4 client implementation for acquiring and renewing IPv4 leases.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use netlib::address::IpConfig;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::DhcpLease;

mod option {
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS_SERVER: u8 = 6;
    pub const REQUESTED_IP: u8 = 50;
    pub const LEASE_TIME: u8 = 51;
    pub const MESSAGE_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const PARAM_REQUEST_LIST: u8 = 55;
    pub const END: u8 = 255;
}

mod message_type {
    pub const DISCOVER: u8 = 1;
    pub const OFFER: u8 = 2;
    pub const REQUEST: u8 = 3;
    pub const ACK: u8 = 5;
}

/// RFC 2131 fixed-header field offsets.
mod field {
    pub const OP: usize = 0;
    pub const HTYPE: usize = 1;
    pub const HLEN: usize = 2;
    pub const HOPS: usize = 3;
    pub const XID: usize = 4;
    pub const FLAGS: usize = 10;
    pub const YIADDR: usize = 16;
    pub const CHADDR: usize = 28;
    /// Total size of the fixed header (up to, but not including, the magic cookie).
    pub const HEADER_LEN: usize = 236;
}

const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const BOOTREQUEST: u8 = 1;
const HTYPE_ETHERNET: u8 = 1;
const HLEN_ETHERNET: u8 = 6;
const FLAG_BROADCAST: u16 = 0x8000;

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;

const DHCP_TIMEOUT_SECS: u64 = 10;
const DEFAULT_LEASE_SECS: u32 = 3600;
const DEFAULT_PREFIX_LEN: u8 = 24;

/// Parsed DHCP options extracted from a response.
pub(crate) struct ParsedOptions {
    pub(crate) message_type: Option<u8>,
    pub(crate) server_id: Option<Ipv4Addr>,
    pub(crate) subnet_mask: Option<Ipv4Addr>,
    pub(crate) router: Option<Ipv4Addr>,
    pub(crate) dns_servers: Vec<Ipv4Addr>,
    pub(crate) lease_time: Option<u32>,
}

/// Build the fixed 236-byte DHCP header + magic cookie.
pub(crate) fn build_header(xid: u32, mac: &[u8; 6]) -> Vec<u8> {
    let mut buf = vec![0u8; field::HEADER_LEN + MAGIC_COOKIE.len()];

    buf[field::OP] = BOOTREQUEST;
    buf[field::HTYPE] = HTYPE_ETHERNET;
    buf[field::HLEN] = HLEN_ETHERNET;
    buf[field::HOPS] = 0;

    buf[field::XID..field::XID + 4].copy_from_slice(&xid.to_be_bytes());
    buf[field::FLAGS..field::FLAGS + 2].copy_from_slice(&FLAG_BROADCAST.to_be_bytes());

    buf[field::CHADDR..field::CHADDR + 6].copy_from_slice(mac);

    buf[field::HEADER_LEN..field::HEADER_LEN + 4].copy_from_slice(&MAGIC_COOKIE);

    buf
}

pub(crate) fn append_param_request_list(msg: &mut Vec<u8>) {
    const REQUESTED_PARAMS: &[u8] = &[
        option::SUBNET_MASK,
        option::ROUTER,
        option::DNS_SERVER,
        option::LEASE_TIME,
    ];
    msg.push(option::PARAM_REQUEST_LIST);
    msg.push(REQUESTED_PARAMS.len() as u8);
    msg.extend_from_slice(REQUESTED_PARAMS);
}

/// Parse the options section of a DHCP response (after the magic cookie).
pub(crate) fn parse_options(options_bytes: &[u8]) -> ParsedOptions {
    let mut parsed = ParsedOptions {
        message_type: None,
        server_id: None,
        subnet_mask: None,
        router: None,
        dns_servers: Vec::new(),
        lease_time: None,
    };

    let mut i = 0;
    while i < options_bytes.len() {
        let code = options_bytes[i];
        if code == option::END {
            break;
        }
        // Pad option (code 0) has no length byte.
        if code == 0 {
            i += 1;
            continue;
        }
        if i + 1 >= options_bytes.len() {
            break;
        }
        let len = options_bytes[i + 1] as usize;
        let data_start = i + 2;
        let data_end = data_start + len;
        if data_end > options_bytes.len() {
            break;
        }
        let data = &options_bytes[data_start..data_end];

        match code {
            option::MESSAGE_TYPE if len == 1 => {
                parsed.message_type = Some(data[0]);
            }
            option::SUBNET_MASK if len == 4 => {
                parsed.subnet_mask = Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
            }
            option::ROUTER if len >= 4 => {
                parsed.router = Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
            }
            option::DNS_SERVER if len >= 4 && len.is_multiple_of(4) => {
                parsed.dns_servers.extend(
                    data.chunks_exact(4)
                        .map(|c| Ipv4Addr::new(c[0], c[1], c[2], c[3])),
                );
            }
            option::LEASE_TIME if len == 4 => {
                parsed.lease_time = Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
            }
            option::SERVER_ID if len == 4 => {
                parsed.server_id = Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
            }
            _ => {}
        }

        i = data_end;
    }

    parsed
}

/// Extract `yiaddr` from a raw DHCP packet.
pub(crate) fn yiaddr(buf: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(
        buf[field::YIADDR],
        buf[field::YIADDR + 1],
        buf[field::YIADDR + 2],
        buf[field::YIADDR + 3],
    )
}

/// Extract `xid` from a raw DHCP packet.
pub(crate) fn xid(buf: &[u8]) -> u32 {
    u32::from_be_bytes([
        buf[field::XID],
        buf[field::XID + 1],
        buf[field::XID + 2],
        buf[field::XID + 3],
    ])
}

/// Validate a received DHCP packet and return its options.
pub(crate) fn validate_response(
    buf: &[u8],
    len: usize,
    expected_xid: u32,
    expected_type: u8,
) -> Result<ParsedOptions> {
    let min_len = field::HEADER_LEN + MAGIC_COOKIE.len();
    if len < min_len {
        bail!("DHCP response too short ({len} bytes)");
    }

    if xid(buf) != expected_xid {
        bail!("DHCP xid mismatch");
    }

    let options_start = field::HEADER_LEN + MAGIC_COOKIE.len();
    let opts = parse_options(&buf[options_start..len]);

    match opts.message_type {
        Some(t) if t == expected_type => Ok(opts),
        Some(t) => bail!("expected DHCP message type {expected_type}, got {t}"),
        None => bail!("DHCP response missing message type option"),
    }
}

/// Runs a full DHCPv4 client exchange on the given interface.
pub async fn run_dhcp_client(interface: &str, mac: &[u8; 6]) -> Result<(IpConfig, DhcpLease)> {
    println!("DHCP: starting on {}", interface);

    let socket = UdpSocket::bind(("0.0.0.0", DHCP_CLIENT_PORT)).await?;
    socket.set_broadcast(true)?;
    netlib::socket::bind_device(&socket, interface)?;

    let xid: u32 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system time before UNIX epoch: {e}"))?
        .as_nanos() as u32;

    // DHCPDISCOVER
    let mut discover = build_header(xid, mac);
    discover.extend(&[option::MESSAGE_TYPE, 1, message_type::DISCOVER]);
    append_param_request_list(&mut discover);
    discover.push(option::END);

    println!("DHCP: sending DISCOVER xid={}", xid);
    socket
        .send_to(&discover, ("255.255.255.255", DHCP_SERVER_PORT))
        .await?;

    // DHCPOFFER
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

    // DHCPREQUEST
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

    // DHCPACK
    let (len, _) = timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        socket.recv_from(&mut buf),
    )
    .await??;
    let ack_opts = validate_response(&buf, len, xid, message_type::ACK)?;
    let ip = yiaddr(&buf);
    println!("DHCP: got ACK yiaddr={}", ip);

    let lease_seconds = ack_opts.lease_time.unwrap_or(DEFAULT_LEASE_SECS);

    let prefix_len: u8 = ack_opts
        .subnet_mask
        .map(|m| m.octets().iter().map(|b| b.count_ones()).sum::<u32>() as u8)
        .unwrap_or(DEFAULT_PREFIX_LEN);

    let ip_cfg = IpConfig {
        address: ip,
        prefix_len,
        gateway: ack_opts.router,
        dns: ack_opts.dns_servers,
    };

    let renewal = lease_seconds / 2;
    let rebind = (lease_seconds * 7) / 8;
    let lease = DhcpLease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(lease_seconds as u64),
        renewal_time: Duration::from_secs(renewal as u64),
        rebind_time: Duration::from_secs(rebind as u64),
    };

    Ok((ip_cfg, lease))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_packet(xid_val: u32, yiaddr_val: Ipv4Addr) -> Vec<u8> {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let mut pkt = build_header(xid_val, &mac);
        pkt[field::YIADDR..field::YIADDR + 4].copy_from_slice(&yiaddr_val.octets());
        pkt
    }

    #[test]
    fn build_header_length() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

        // ACT
        let hdr = build_header(0x12345678, &mac);

        // ASSERT
        assert_eq!(hdr.len(), field::HEADER_LEN + MAGIC_COOKIE.len());
    }

    #[test]
    fn build_header_fields() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

        // ACT
        let hdr = build_header(0xDEADBEEF, &mac);

        // ASSERT
        assert_eq!(hdr[field::OP], BOOTREQUEST);
        assert_eq!(hdr[field::HTYPE], HTYPE_ETHERNET);
        assert_eq!(hdr[field::HLEN], HLEN_ETHERNET);
        assert_eq!(hdr[field::HOPS], 0);
        let xid_bytes = &hdr[field::XID..field::XID + 4];
        assert_eq!(xid_bytes, &0xDEADBEEFu32.to_be_bytes());
        assert_eq!(&hdr[field::CHADDR..field::CHADDR + 6], &mac);
        let cookie = &hdr[field::HEADER_LEN..field::HEADER_LEN + 4];
        assert_eq!(cookie, &MAGIC_COOKIE);
    }

    #[test]
    fn build_header_broadcast_flag_set() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let hdr = build_header(1, &mac);

        // ASSERT
        let flags = u16::from_be_bytes([hdr[field::FLAGS], hdr[field::FLAGS + 1]]);
        assert_eq!(flags, FLAG_BROADCAST);
    }

    #[test]
    fn append_param_request_list_content() {
        // ARRANGE
        let mut msg = Vec::new();

        // ACT
        append_param_request_list(&mut msg);

        // ASSERT
        assert_eq!(msg[0], option::PARAM_REQUEST_LIST);
        assert_eq!(msg[1], 4);
        assert_eq!(msg[2], option::SUBNET_MASK);
        assert_eq!(msg[3], option::ROUTER);
        assert_eq!(msg[4], option::DNS_SERVER);
        assert_eq!(msg[5], option::LEASE_TIME);
    }

    #[test]
    fn parse_options_empty() {
        // ACT
        let opts = parse_options(&[]);

        // ASSERT
        assert!(opts.message_type.is_none());
        assert!(opts.server_id.is_none());
        assert!(opts.subnet_mask.is_none());
        assert!(opts.router.is_none());
        assert!(opts.dns_servers.is_empty());
        assert!(opts.lease_time.is_none());
    }

    #[test]
    fn parse_options_end_marker() {
        // ARRANGE
        let data = [option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert!(opts.message_type.is_none());
    }

    #[test]
    fn parse_options_message_type() {
        // ARRANGE
        let data = [option::MESSAGE_TYPE, 1, message_type::OFFER, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
    }

    #[test]
    fn parse_options_subnet_mask() {
        // ARRANGE
        let data = [option::SUBNET_MASK, 4, 255, 255, 255, 0, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.subnet_mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
    }

    #[test]
    fn parse_options_router() {
        // ARRANGE
        let data = [option::ROUTER, 4, 10, 0, 0, 1, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.router, Some(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn parse_options_dns_servers_two() {
        // ARRANGE
        let data = [option::DNS_SERVER, 8, 8, 8, 8, 8, 8, 8, 4, 4, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.dns_servers.len(), 2);
        assert_eq!(opts.dns_servers[0], Ipv4Addr::new(8, 8, 8, 8));
        assert_eq!(opts.dns_servers[1], Ipv4Addr::new(8, 8, 4, 4));
    }

    #[test]
    fn parse_options_dns_odd_length_ignored() {
        // ARRANGE
        let data = [option::DNS_SERVER, 5, 8, 8, 8, 8, 0, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert!(opts.dns_servers.is_empty());
    }

    #[test]
    fn parse_options_lease_time() {
        // ARRANGE
        let data = [option::LEASE_TIME, 4, 0, 0, 0x0E, 0x10, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.lease_time, Some(3600));
    }

    #[test]
    fn parse_options_server_id() {
        // ARRANGE
        let data = [option::SERVER_ID, 4, 192, 168, 1, 1, option::END];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.server_id, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn parse_options_pad_bytes_skipped() {
        // ARRANGE
        let data = [
            0,
            0,
            option::MESSAGE_TYPE,
            1,
            message_type::ACK,
            option::END,
        ];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::ACK));
    }

    #[test]
    fn parse_options_truncated_data_safe() {
        // ARRANGE
        let data = [option::LEASE_TIME, 4, 0, 0];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert!(opts.lease_time.is_none());
    }

    #[test]
    fn parse_options_all_fields() {
        // ARRANGE
        let mut data = Vec::new();
        data.extend([option::MESSAGE_TYPE, 1, message_type::ACK]);
        data.extend([option::SUBNET_MASK, 4, 255, 255, 0, 0]);
        data.extend([option::ROUTER, 4, 10, 0, 0, 1]);
        data.extend([option::DNS_SERVER, 4, 1, 1, 1, 1]);
        data.extend([option::LEASE_TIME, 4, 0, 0, 0x1C, 0x20]);
        data.extend([option::SERVER_ID, 4, 192, 168, 0, 1]);
        data.push(option::END);

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::ACK));
        assert_eq!(opts.subnet_mask, Some(Ipv4Addr::new(255, 255, 0, 0)));
        assert_eq!(opts.router, Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(opts.dns_servers, vec![Ipv4Addr::new(1, 1, 1, 1)]);
        assert_eq!(opts.lease_time, Some(7200));
        assert_eq!(opts.server_id, Some(Ipv4Addr::new(192, 168, 0, 1)));
    }

    #[test]
    fn yiaddr_extraction() {
        // ARRANGE
        let pkt = make_minimal_packet(1, Ipv4Addr::new(10, 0, 0, 42));

        // ACT / ASSERT
        assert_eq!(yiaddr(&pkt), Ipv4Addr::new(10, 0, 0, 42));
    }

    #[test]
    fn xid_extraction() {
        // ARRANGE
        let pkt = make_minimal_packet(0xCAFEBABE, Ipv4Addr::UNSPECIFIED);

        // ACT / ASSERT
        assert_eq!(xid(&pkt), 0xCAFEBABE);
    }

    #[test]
    fn validate_response_too_short() {
        // ARRANGE
        let buf = [0u8; 100];

        // ACT
        let result = validate_response(&buf, 100, 0, message_type::OFFER);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_response_xid_mismatch() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111, Ipv4Addr::UNSPECIFIED);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::OFFER, option::END]);
        let len = pkt.len();

        // ACT
        let result = validate_response(&pkt, len, 0x2222, message_type::OFFER);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_response_wrong_message_type() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111, Ipv4Addr::UNSPECIFIED);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::ACK, option::END]);
        let len = pkt.len();

        // ACT
        let result = validate_response(&pkt, len, 0x1111, message_type::OFFER);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_response_missing_message_type() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111, Ipv4Addr::UNSPECIFIED);
        pkt.push(option::END);
        let len = pkt.len();

        // ACT
        let result = validate_response(&pkt, len, 0x1111, message_type::OFFER);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_response_success() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0xABCD, Ipv4Addr::new(10, 0, 0, 1));
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::OFFER]);
        pkt.extend([option::LEASE_TIME, 4, 0, 0, 0x0E, 0x10]);
        pkt.push(option::END);
        let len = pkt.len();

        // ACT
        let opts = validate_response(&pkt, len, 0xABCD, message_type::OFFER).unwrap();

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
        assert_eq!(opts.lease_time, Some(3600));
    }
}
