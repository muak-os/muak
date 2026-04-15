//! DHCPv4 option encoding/decoding, XID generation, and lease construction.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use ring::rand::{SecureRandom, SystemRandom};

use super::DhcpLease;
use super::packet::{
    DEFAULT_LEASE_SECS, DEFAULT_PREFIX_LEN, MAGIC_COOKIE, field, message_type, option,
};

/// Indicates the DHCP server sent a NAK, requiring a return to INIT state.
#[derive(Debug)]
pub struct DhcpNak;

impl std::fmt::Display for DhcpNak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DHCP server sent NAK")
    }
}

impl std::error::Error for DhcpNak {}

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

/// Generates a cryptographically random 32-bit DHCP transaction ID.
pub(crate) fn generate_xid() -> Result<u32> {
    let rng = SystemRandom::new();
    let mut buf = [0u8; 4];
    rng.fill(&mut buf)
        .map_err(|_| anyhow::anyhow!("failed to generate random DHCP xid"))?;
    Ok(u32::from_be_bytes(buf))
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

    let mut i = 0;
    while i < options_bytes.len() {
        let code = options_bytes[i];
        if code == option::END {
            break;
        }
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

/// Validates a received DHCP packet, detecting NAK responses.
pub(crate) fn validate_response(
    buf: &[u8],
    len: usize,
    expected_xid: u32,
    expected_type: u8,
) -> Result<ParsedOptions> {
    let min_len = field::HEADER_LEN + MAGIC_COOKIE.len();
    if len < min_len {
        anyhow::bail!("DHCP response too short ({len} bytes)");
    }

    if super::packet::xid(buf) != expected_xid {
        anyhow::bail!("DHCP xid mismatch");
    }

    let options_start = field::HEADER_LEN + MAGIC_COOKIE.len();
    let opts = parse_options(&buf[options_start..len]);

    match opts.message_type {
        Some(t) if t == message_type::NAK => Err(DhcpNak.into()),
        Some(t) if t == expected_type => Ok(opts),
        Some(t) => anyhow::bail!("expected DHCP message type {expected_type}, got {t}"),
        None => anyhow::bail!("DHCP response missing message type option"),
    }
}

/// Constructs a `DhcpLease` from a validated ACK response.
pub(crate) fn build_lease_from_ack(
    ip: Ipv4Addr,
    server_ip: Ipv4Addr,
    opts: &ParsedOptions,
) -> Result<DhcpLease> {
    let lease_seconds = opts.lease_time.unwrap_or(DEFAULT_LEASE_SECS);
    let prefix_len = opts
        .subnet_mask
        .map(|m| m.octets().iter().map(|b| b.count_ones()).sum::<u32>() as u8)
        .unwrap_or(DEFAULT_PREFIX_LEN);

    let renewal = lease_seconds / 2;
    let rebind = (lease_seconds * 7) / 8;

    Ok(DhcpLease {
        obtained_at: SystemTime::now(),
        lease_time: Duration::from_secs(lease_seconds as u64),
        renewal_time: Duration::from_secs(renewal as u64),
        rebind_time: Duration::from_secs(rebind as u64),
        server_ip,
        assigned_ip: ip,
        prefix_len,
        gateway: opts.router,
        dns_servers: opts.dns_servers.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dhcp::packet::build_header;

    fn make_minimal_packet(xid_val: u32, yiaddr_val: Ipv4Addr) -> Vec<u8> {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let mut pkt = build_header(xid_val, &mac);
        pkt[field::YIADDR..field::YIADDR + 4].copy_from_slice(&yiaddr_val.octets());
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
        let xid = generate_xid().expect("should generate xid");

        // ASSERT
        let _ = xid;
    }

    #[test]
    fn generate_xid_produces_different_values() {
        // ACT
        let a = generate_xid().expect("xid a");
        let b = generate_xid().expect("xid b");

        // ASSERT
        assert_ne!(a, b);
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
        )
        .expect("should build lease");

        // ASSERT
        assert_eq!(lease.assigned_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(lease.server_ip, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(lease.prefix_len, DEFAULT_PREFIX_LEN);
        assert_eq!(
            lease.lease_time,
            Duration::from_secs(DEFAULT_LEASE_SECS as u64)
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
        )
        .expect("should build lease");

        // ASSERT
        assert_eq!(lease.prefix_len, 24);
        assert_eq!(lease.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(lease.dns_servers, vec![Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(lease.lease_time, Duration::from_secs(7200));
        assert_eq!(lease.renewal_time, Duration::from_secs(3600));
        assert_eq!(lease.rebind_time, Duration::from_secs(6300));
    }
}
