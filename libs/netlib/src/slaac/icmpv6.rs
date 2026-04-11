//! ICMPv6 packet construction and parsing for stateless address autoconfiguration (RFC 4861).

use std::array::TryFromSliceError;
use std::net::Ipv6Addr;

use thiserror::Error;

pub const ICMPV6_ROUTER_SOLICITATION: u8 = 133;
pub const ICMPV6_ROUTER_ADVERTISEMENT: u8 = 134;

const ND_OPT_SOURCE_LL_ADDR: u8 = 1;
const ND_OPT_PREFIX_INFO: u8 = 3;
const ND_OPT_RDNSS: u8 = 25;

#[derive(Debug, Clone)]
pub struct PrefixInfo {
    pub prefix: Ipv6Addr,
    pub prefix_len: u8,
    pub autonomous: bool,
    pub valid_lifetime: u32,
    pub preferred_lifetime: u32,
}

#[derive(Debug, Clone)]
pub struct RouterAdvertisement {
    pub hop_limit: u8,
    pub managed_flag: bool,
    pub other_flag: bool,
    pub router_lifetime: u16,
    pub source: Ipv6Addr,
    pub prefixes: Vec<PrefixInfo>,
    pub dns_servers: Vec<Ipv6Addr>,
    pub dns_lifetime: u32,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("RA too short: {0} bytes")]
    TooShort(usize),
    #[error("not a Router Advertisement: type={0}")]
    WrongType(u8),
    #[error("malformed address field in RA option: {0}")]
    MalformedAddress(#[from] TryFromSliceError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Constructs an ICMPv6 Router Solicitation packet with a source link-layer address option.
pub fn build_router_solicitation(mac: &[u8; 6]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(16);

    pkt.push(ICMPV6_ROUTER_SOLICITATION);
    pkt.push(0);
    pkt.extend([0, 0]);

    pkt.extend([0, 0, 0, 0]);

    pkt.push(ND_OPT_SOURCE_LL_ADDR);
    pkt.push(1);
    pkt.extend(mac);

    pkt
}

/// Parses a raw ICMPv6 Router Advertisement into structured prefix, DNS and flag data.
pub fn parse_router_advertisement(data: &[u8], source: Ipv6Addr) -> Result<RouterAdvertisement> {
    if data.len() < 16 {
        return Err(Error::TooShort(data.len()));
    }

    if data[0] != ICMPV6_ROUTER_ADVERTISEMENT {
        return Err(Error::WrongType(data[0]));
    }

    let hop_limit = data[4];
    let flags = data[5];
    let managed_flag = (flags & 0x80) != 0;
    let other_flag = (flags & 0x40) != 0;
    let router_lifetime = u16::from_be_bytes([data[6], data[7]]);

    let mut ra = RouterAdvertisement {
        hop_limit,
        managed_flag,
        other_flag,
        router_lifetime,
        source,
        prefixes: Vec::new(),
        dns_servers: Vec::new(),
        dns_lifetime: 0,
    };

    let mut pos = 16;
    while pos + 2 <= data.len() {
        let opt_type = data[pos];
        let opt_len_units = data[pos + 1] as usize;
        if opt_len_units == 0 {
            break;
        }
        let opt_len = opt_len_units * 8;
        if pos + opt_len > data.len() {
            break;
        }

        match opt_type {
            ND_OPT_PREFIX_INFO if opt_len >= 32 => {
                let prefix_len = data[pos + 2];
                let flags = data[pos + 3];
                let autonomous = (flags & 0x40) != 0;
                let valid_lifetime = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]);
                let preferred_lifetime = u32::from_be_bytes([
                    data[pos + 8],
                    data[pos + 9],
                    data[pos + 10],
                    data[pos + 11],
                ]);
                let prefix_bytes: [u8; 16] = data[pos + 16..pos + 32].try_into()?;
                let prefix = Ipv6Addr::from(prefix_bytes);

                ra.prefixes.push(PrefixInfo {
                    prefix,
                    prefix_len,
                    autonomous,
                    valid_lifetime,
                    preferred_lifetime,
                });
            }
            ND_OPT_RDNSS if opt_len >= 24 => {
                ra.dns_lifetime = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]);
                parse_rdnss_addresses(data, pos, opt_len, &mut ra.dns_servers)?;
            }
            _ => {}
        }

        pos += opt_len;
    }

    Ok(ra)
}

fn parse_rdnss_addresses(
    data: &[u8],
    pos: usize,
    opt_len: usize,
    servers: &mut Vec<Ipv6Addr>,
) -> Result<()> {
    let addr_count = (opt_len - 8) / 16;
    for i in 0..addr_count {
        let start = pos + 8 + i * 16;
        let addr_bytes: [u8; 16] = data[start..start + 16].try_into()?;
        servers.push(Ipv6Addr::from(addr_bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ra_base(hop_limit: u8, flags: u8, router_lifetime: u16) -> Vec<u8> {
        let mut data = vec![0u8; 16];
        data[0] = ICMPV6_ROUTER_ADVERTISEMENT;
        data[1] = 0;
        data[4] = hop_limit;
        data[5] = flags;
        data[6..8].copy_from_slice(&router_lifetime.to_be_bytes());
        data
    }

    fn append_prefix_option(
        data: &mut Vec<u8>,
        prefix_len: u8,
        autonomous: bool,
        valid: u32,
        preferred: u32,
        prefix: Ipv6Addr,
    ) {
        let start = data.len();
        data.resize(start + 32, 0);
        data[start] = ND_OPT_PREFIX_INFO;
        data[start + 1] = 4;
        data[start + 2] = prefix_len;
        data[start + 3] = if autonomous { 0x40 } else { 0x00 };
        data[start + 4..start + 8].copy_from_slice(&valid.to_be_bytes());
        data[start + 8..start + 12].copy_from_slice(&preferred.to_be_bytes());
        data[start + 16..start + 32].copy_from_slice(&prefix.octets());
    }

    fn append_rdnss_option(data: &mut Vec<u8>, lifetime: u32, servers: &[Ipv6Addr]) {
        let opt_len_units = (1 + servers.len() * 2) as u8;
        let start = data.len();
        let byte_len = opt_len_units as usize * 8;
        data.resize(start + byte_len, 0);
        data[start] = ND_OPT_RDNSS;
        data[start + 1] = opt_len_units;
        data[start + 4..start + 8].copy_from_slice(&lifetime.to_be_bytes());
        for (i, server) in servers.iter().enumerate() {
            let off = start + 8 + i * 16;
            data[off..off + 16].copy_from_slice(&server.octets());
        }
    }

    #[test]
    fn build_router_solicitation_structure() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT
        let pkt = build_router_solicitation(&mac);

        // ASSERT
        assert_eq!(pkt[0], ICMPV6_ROUTER_SOLICITATION);
        assert_eq!(pkt[1], 0);
        assert_eq!(pkt[8], ND_OPT_SOURCE_LL_ADDR);
        assert_eq!(pkt[9], 1);
        assert_eq!(&pkt[10..16], &mac);
    }

    #[test]
    fn build_router_solicitation_length() {
        // ARRANGE
        let mac = [0; 6];

        // ACT
        let pkt = build_router_solicitation(&mac);

        // ASSERT
        assert_eq!(pkt.len(), 16);
    }

    #[test]
    fn parse_ra_minimal() {
        // ARRANGE
        let data = make_ra_base(64, 0xC0, 1800);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.hop_limit, 64);
        assert!(ra.managed_flag);
        assert!(ra.other_flag);
        assert_eq!(ra.router_lifetime, 1800);
        assert_eq!(ra.source, source);
        assert!(ra.prefixes.is_empty());
        assert!(ra.dns_servers.is_empty());
    }

    #[test]
    fn parse_ra_flags_managed_only() {
        // ARRANGE
        let data = make_ra_base(0, 0x80, 0);
        let source = "fe80::2".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert!(ra.managed_flag);
        assert!(!ra.other_flag);
    }

    #[test]
    fn parse_ra_flags_other_only() {
        // ARRANGE
        let data = make_ra_base(0, 0x40, 0);
        let source = "fe80::2".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert!(!ra.managed_flag);
        assert!(ra.other_flag);
    }

    #[test]
    fn parse_ra_flags_none() {
        // ARRANGE
        let data = make_ra_base(0, 0x00, 0);
        let source = "fe80::2".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert!(!ra.managed_flag);
        assert!(!ra.other_flag);
    }

    #[test]
    fn parse_ra_too_short() {
        // ARRANGE
        let data = vec![ICMPV6_ROUTER_ADVERTISEMENT, 0, 0, 0];
        let source = "fe80::1".parse().expect("valid address");

        // ACT / ASSERT
        assert!(parse_router_advertisement(&data, source).is_err());
    }

    #[test]
    fn parse_ra_wrong_type() {
        // ARRANGE
        let mut data = make_ra_base(0, 0, 0);
        data[0] = ICMPV6_ROUTER_SOLICITATION;
        let source = "fe80::1".parse().expect("valid address");

        // ACT / ASSERT
        assert!(parse_router_advertisement(&data, source).is_err());
    }

    #[test]
    fn parse_ra_with_prefix() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        let prefix: Ipv6Addr = "2001:db8::".parse().expect("valid prefix");
        append_prefix_option(&mut data, 64, true, 7200, 3600, prefix);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.prefixes.len(), 1);
        assert_eq!(ra.prefixes[0].prefix, prefix);
        assert_eq!(ra.prefixes[0].prefix_len, 64);
        assert!(ra.prefixes[0].autonomous);
        assert_eq!(ra.prefixes[0].valid_lifetime, 7200);
        assert_eq!(ra.prefixes[0].preferred_lifetime, 3600);
    }

    #[test]
    fn parse_ra_prefix_non_autonomous() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        let prefix: Ipv6Addr = "2001:db8:1::".parse().expect("valid prefix");
        append_prefix_option(&mut data, 48, false, 86400, 43200, prefix);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.prefixes.len(), 1);
        assert!(!ra.prefixes[0].autonomous);
        assert_eq!(ra.prefixes[0].prefix_len, 48);
    }

    #[test]
    fn parse_ra_multiple_prefixes() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        append_prefix_option(
            &mut data,
            64,
            true,
            7200,
            3600,
            "2001:db8::".parse().expect("valid prefix"),
        );
        append_prefix_option(
            &mut data,
            48,
            true,
            86400,
            43200,
            "2001:db8:1::".parse().expect("valid prefix"),
        );
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.prefixes.len(), 2);
    }

    #[test]
    fn parse_ra_with_rdnss() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        let dns1: Ipv6Addr = "2620:fe::fe".parse().expect("valid address");
        let dns2: Ipv6Addr = "2620:fe::9".parse().expect("valid address");
        append_rdnss_option(&mut data, 3600, &[dns1, dns2]);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.dns_servers.len(), 2);
        assert_eq!(ra.dns_servers[0], dns1);
        assert_eq!(ra.dns_servers[1], dns2);
        assert_eq!(ra.dns_lifetime, 3600);
    }

    #[test]
    fn parse_ra_with_prefix_and_rdnss() {
        // ARRANGE
        let mut data = make_ra_base(64, 0x80, 1800);
        append_prefix_option(
            &mut data,
            64,
            true,
            7200,
            3600,
            "2001:db8::".parse().expect("valid prefix"),
        );
        let dns: Ipv6Addr = "2620:fe::fe".parse().expect("valid address");
        append_rdnss_option(&mut data, 600, &[dns]);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.prefixes.len(), 1);
        assert_eq!(ra.dns_servers.len(), 1);
        assert!(ra.managed_flag);
    }

    #[test]
    fn parse_ra_zero_length_option_terminates() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        data.push(ND_OPT_PREFIX_INFO);
        data.push(0);
        data.extend([0u8; 30]);
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert!(ra.prefixes.is_empty());
    }

    #[test]
    fn parse_ra_unknown_option_skipped() {
        // ARRANGE
        let mut data = make_ra_base(64, 0, 1800);
        data.push(99);
        data.push(1);
        data.extend([0u8; 6]);
        append_prefix_option(
            &mut data,
            64,
            true,
            7200,
            3600,
            "2001:db8::".parse().expect("valid prefix"),
        );
        let source = "fe80::1".parse().expect("valid address");

        // ACT
        let ra = parse_router_advertisement(&data, source).expect("should parse");

        // ASSERT
        assert_eq!(ra.prefixes.len(), 1);
    }
}
