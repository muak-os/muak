//! `DHCPv4` option parsing (RFC 2132).

use core::net::Ipv4Addr;

use super::option;
use crate::ParsedOptions;

/// Parses the options section of a DHCP response (after the magic cookie).
#[must_use = "the parsed options hold the response fields"]
pub fn parse(options_bytes: &[u8]) -> ParsedOptions {
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
                        .map(|chunk| Ipv4Addr::from(*chunk)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_type;

    #[test]
    fn parse_empty() {
        // ACT
        let opts = parse(&[]);

        // ASSERT
        assert!(opts.message_type.is_none());
        assert!(opts.server_id.is_none());
        assert!(opts.subnet_mask.is_none());
        assert!(opts.router.is_none());
        assert_eq!(opts.dns_servers, Vec::<Ipv4Addr>::new());
        assert!(opts.lease_time.is_none());
    }

    #[test]
    fn parse_end_marker() {
        // ARRANGE
        let data = [option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert!(opts.message_type.is_none());
    }

    #[test]
    fn parse_message_type() {
        // ARRANGE
        let data = [option::MESSAGE_TYPE, 1, message_type::OFFER, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
    }

    #[test]
    fn parse_subnet_mask() {
        // ARRANGE
        let data = [option::SUBNET_MASK, 4, 255, 255, 255, 0, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.subnet_mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
    }

    #[test]
    fn parse_router() {
        // ARRANGE
        let data = [option::ROUTER, 4, 10, 0, 0, 1, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.router, Some(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn parse_dns_servers_two() {
        // ARRANGE
        let data = [option::DNS_SERVER, 8, 8, 8, 8, 8, 8, 8, 4, 4, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.dns_servers.len(), 2);
        assert_eq!(opts.dns_servers.first(), Some(&Ipv4Addr::new(8, 8, 8, 8)));
        assert_eq!(opts.dns_servers.get(1), Some(&Ipv4Addr::new(8, 8, 4, 4)));
    }

    #[test]
    fn parse_dns_odd_length_ignored() {
        // ARRANGE
        let data = [option::DNS_SERVER, 5, 8, 8, 8, 8, 0, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.dns_servers, Vec::<Ipv4Addr>::new());
    }

    #[test]
    fn parse_lease_time() {
        // ARRANGE
        let data = [option::LEASE_TIME, 4, 0, 0, 0x0E, 0x10, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.lease_time, Some(3600));
    }

    #[test]
    fn parse_server_id() {
        // ARRANGE
        let data = [option::SERVER_ID, 4, 192, 168, 1, 1, option::END];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.server_id, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn parse_pad_bytes_skipped() {
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
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::ACK));
    }

    #[test]
    fn parse_truncated_data_safe() {
        // ARRANGE
        let data = [option::LEASE_TIME, 4, 0, 0];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert!(opts.lease_time.is_none());
    }

    #[test]
    fn parse_truncated_length_field() {
        // ARRANGE
        let data = [option::ROUTER];

        // ACT
        let opts = parse(&data);

        // ASSERT
        assert!(opts.router.is_none());
    }

    #[test]
    fn parse_unknown_option_code_skipped() {
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
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
    }

    #[test]
    fn parse_all_fields() {
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
        let opts = parse(&data);

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::ACK));
        assert_eq!(opts.subnet_mask, Some(Ipv4Addr::new(255, 255, 0, 0)));
        assert_eq!(opts.router, Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(opts.dns_servers, vec![Ipv4Addr::new(1, 1, 1, 1)]);
        assert_eq!(opts.lease_time, Some(7200));
        assert_eq!(opts.server_id, Some(Ipv4Addr::new(192, 168, 0, 1)));
    }
}
