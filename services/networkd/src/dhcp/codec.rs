//! `DHCPv4` option encoding/decoding, XID generation, and lease construction.

use core::error::Error;
use core::fmt;
use core::net::Ipv4Addr;
use core::time::Duration;
use std::time::SystemTime;

use anyhow::Result;

use super::Lease;
use super::packet::{
    DEFAULT_LEASE_SECS, DEFAULT_PREFIX_LEN, MAGIC_COOKIE, field, message_type, option,
};

/// Indicates the DHCP server sent a NAK, requiring a return to INIT state.
#[derive(Debug)]
pub struct DhcpNak;

impl fmt::Display for DhcpNak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DHCP server sent NAK")
    }
}

impl Error for DhcpNak {}

/// Parsed DHCP options extracted from a response.
#[derive(Debug)]
pub(crate) struct ParsedOptions {
    pub(crate) message_type: Option<u8>,
    pub(crate) server_id: Option<Ipv4Addr>,
    pub(crate) subnet_mask: Option<Ipv4Addr>,
    pub(crate) router: Option<Ipv4Addr>,
    pub(crate) dns_servers: Vec<Ipv4Addr>,
    pub(crate) lease_time: Option<u32>,
}

/// Validates a received DHCP packet, detecting NAK responses.
///
/// # Errors
///
/// Returns an error if the packet is too short, the xid does not match, or the
/// message type is unexpected.
pub(crate) fn validate_response(
    buf: &[u8],
    len: usize,
    expected_xid: u32,
    expected_type: u8,
) -> Result<ParsedOptions> {
    let min_len = field::HEADER_LEN.saturating_add(MAGIC_COOKIE.len());
    if len < min_len {
        anyhow::bail!("DHCP response too short ({len} bytes)");
    }

    if super::packet::xid(buf) != expected_xid {
        anyhow::bail!("DHCP xid mismatch");
    }

    let options_start = field::HEADER_LEN.saturating_add(MAGIC_COOKIE.len());
    let opts_bytes = buf
        .get(options_start..len)
        .ok_or_else(|| anyhow::anyhow!("DHCP response truncated"))?;
    let opts = parse_options(opts_bytes);

    match opts.message_type {
        Some(msg_type) if msg_type == message_type::NAK => Err(DhcpNak.into()),
        Some(msg_type) if msg_type == expected_type => Ok(opts),
        Some(msg_type) => {
            anyhow::bail!("expected DHCP message type {expected_type}, got {msg_type}")
        }
        None => anyhow::bail!("DHCP response missing message type option"),
    }
}

/// Parses the options section of a DHCP response (after the magic cookie).
pub(crate) fn parse_options(options_bytes: &[u8]) -> ParsedOptions {
    let mut parsed = ParsedOptions {
        message_type: None,
        server_id: None,
        subnet_mask: None,
        router: None,
        dns_servers: Vec::new(),
        lease_time: None,
    };

    let mut cursor = 0;
    while cursor < options_bytes.len() {
        let Some(&code) = options_bytes.get(cursor) else {
            break;
        };
        if code == option::END {
            break;
        }
        if code == 0 {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let Some(&length) = options_bytes.get(cursor.saturating_add(1)) else {
            break;
        };
        let data_start = cursor.saturating_add(2);
        let data_end = data_start.saturating_add(usize::from(length));
        let Some(data) = options_bytes.get(data_start..data_end) else {
            break;
        };

        match code {
            option::MESSAGE_TYPE if length == 1 => {
                parsed.message_type = data.first().copied();
            }
            option::SUBNET_MASK if length == 4 => {
                parsed.subnet_mask = data
                    .first_chunk::<4>()
                    .map(|octets| Ipv4Addr::from(*octets));
            }
            option::ROUTER if length >= 4 => {
                parsed.router = data
                    .first_chunk::<4>()
                    .map(|octets| Ipv4Addr::from(*octets));
            }
            option::DNS_SERVER if length >= 4 && length.is_multiple_of(4) => {
                parsed.dns_servers.extend(
                    data.as_chunks::<4>()
                        .0
                        .iter()
                        .filter_map(|chunk| ipv4_from_chunk(chunk)),
                );
            }
            option::LEASE_TIME if length == 4 => {
                parsed.lease_time = data
                    .first_chunk::<4>()
                    .map(|octets| u32::from_be_bytes(*octets));
            }
            option::SERVER_ID if length == 4 => {
                parsed.server_id = data
                    .first_chunk::<4>()
                    .map(|octets| Ipv4Addr::from(*octets));
            }
            _ => {}
        }

        cursor = data_end;
    }

    parsed
}

/// Constructs a `Lease` from a validated ACK response.
pub(crate) fn build_lease_from_ack(
    ip: Ipv4Addr,
    server_ip: Ipv4Addr,
    opts: &ParsedOptions,
) -> Lease {
    let lease_seconds = opts.lease_time.unwrap_or(DEFAULT_LEASE_SECS);
    let prefix_len = opts.subnet_mask.map_or(DEFAULT_PREFIX_LEN, |mask| {
        let ones = mask.octets().into_iter().map(u8::count_ones).sum::<u32>();
        u8::try_from(ones).unwrap_or(DEFAULT_PREFIX_LEN)
    });

    let renewal = lease_seconds.saturating_div(2);
    let rebind = lease_seconds.saturating_mul(7).saturating_div(8);

    Lease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(u64::from(lease_seconds)),
        renewal_time: Duration::from_secs(u64::from(renewal)),
        rebind_time: Duration::from_secs(u64::from(rebind)),
        server_ip,
        assigned_ip: ip,
        prefix_len,
        gateway: opts.router,
        dns_servers: opts.dns_servers.clone(),
    }
}

/// Generates a cryptographically random 32-bit DHCP transaction ID.
pub(crate) fn generate_xid() -> Result<u32> {
    let mut buf = [0_u8; 4];
    getrandom::fill(&mut buf)
        .map_err(|error| anyhow::anyhow!("failed to generate random DHCP xid: {error}"))?;

    Ok(u32::from_be_bytes(buf))
}

/// Converts a 4-byte chunk into an IPv4 address, if it is exactly 4 bytes long.
fn ipv4_from_chunk(chunk: &[u8]) -> Option<Ipv4Addr> {
    chunk
        .first_chunk::<4>()
        .map(|octets| Ipv4Addr::from(*octets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dhcp::packet::build_header;

    fn make_minimal_packet(xid_val: u32, yiaddr_val: Ipv4Addr) -> Vec<u8> {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let mut pkt = build_header(xid_val, mac);
        if let Some(slot) = pkt.get_mut(field::YIADDR..field::YIADDR + 4) {
            slot.copy_from_slice(&yiaddr_val.octets());
        }
        pkt
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
        assert_eq!(opts.dns_servers.first(), Some(&Ipv4Addr::new(8, 8, 8, 8)));
        assert_eq!(opts.dns_servers.get(1), Some(&Ipv4Addr::new(8, 8, 4, 4)));
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
    fn parse_options_truncated_length_field() {
        // ARRANGE
        let data = [option::ROUTER];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert!(opts.router.is_none());
    }

    #[test]
    fn parse_options_unknown_option_code_skipped() {
        // ARRANGE
        let data = [
            99,
            1,
            0x00,
            option::MESSAGE_TYPE,
            1,
            message_type::OFFER,
            option::END,
        ];

        // ACT
        let opts = parse_options(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
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
    fn validate_response_too_short() {
        // ARRANGE
        let buf = [0_u8; 100];

        // ACT
        let result = validate_response(&buf, 100, 0, message_type::OFFER);

        // ASSERT
        result.unwrap_err();
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
        result.unwrap_err();
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
        result.unwrap_err();
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
        result.unwrap_err();
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
        let opts =
            validate_response(&pkt, len, 0xABCD, message_type::OFFER).expect("should validate");

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
        assert_eq!(opts.lease_time, Some(3600));
    }

    #[test]
    fn validate_response_nak_returns_dhcp_nak_error() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1234, Ipv4Addr::UNSPECIFIED);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::NAK, option::END]);
        let len = pkt.len();

        // ACT
        let result = validate_response(&pkt, len, 0x1234, message_type::ACK);

        // ASSERT
        assert!(result.is_err());
        assert!(result.unwrap_err().downcast_ref::<DhcpNak>().is_some());
    }

    #[test]
    fn generate_xid_produces_value() {
        // ACT
        let _xid = generate_xid().expect("should generate xid");
    }

    #[test]
    fn generate_xid_produces_different_values() {
        // ACT
        let first_xid = generate_xid().expect("xid a");
        let second_xid = generate_xid().expect("xid b");

        // ASSERT
        assert_ne!(first_xid, second_xid);
    }

    #[test]
    fn build_lease_from_ack_defaults() {
        // ARRANGE
        let opts = ParsedOptions {
            message_type: Some(message_type::ACK),
            server_id: Some(Ipv4Addr::new(192, 168, 1, 1)),
            subnet_mask: None,
            router: None,
            dns_servers: Vec::new(),
            lease_time: None,
        };

        // ACT
        let lease = build_lease_from_ack(
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(192, 168, 1, 1),
            &opts,
        );

        // ASSERT
        assert_eq!(lease.assigned_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(lease.server_ip, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(lease.prefix_len, DEFAULT_PREFIX_LEN);
        assert_eq!(
            lease.lease_time,
            Duration::from_secs(u64::from(DEFAULT_LEASE_SECS))
        );
        assert!(lease.gateway.is_none());
        assert!(lease.dns_servers.is_empty());
    }

    #[test]
    fn build_lease_from_ack_with_all_options() {
        // ARRANGE
        let opts = ParsedOptions {
            message_type: Some(message_type::ACK),
            server_id: Some(Ipv4Addr::new(192, 168, 1, 1)),
            subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
            lease_time: Some(7200),
        };

        // ACT
        let lease = build_lease_from_ack(
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(192, 168, 1, 1),
            &opts,
        );

        // ASSERT
        assert_eq!(lease.prefix_len, 24);
        assert_eq!(lease.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(lease.dns_servers, vec![Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(lease.lease_time, Duration::from_hours(2));
        assert_eq!(lease.renewal_time, Duration::from_hours(1));
        assert_eq!(lease.rebind_time, Duration::from_mins(105));
    }
}
