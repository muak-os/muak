//! `ICMPv6` packet construction and parsing for stateless address autoconfiguration (RFC 4861).

use core::net::Ipv6Addr;

use thiserror::Error;

/// `ICMPv6` Router Solicitation type code.
pub const ICMPV6_ROUTER_SOLICITATION: u8 = 133;
/// `ICMPv6` Router Advertisement type code.
pub const ICMPV6_ROUTER_ADVERTISEMENT: u8 = 134;

const ND_OPT_SOURCE_LL_ADDR: u8 = 1;
const ND_OPT_PREFIX_INFO: u8 = 3;
const ND_OPT_RDNSS: u8 = 25;

const RA_HEADER_LEN: usize = 16;
const OPTION_HEADER_LEN: usize = 2;
const OPTION_UNIT_LEN: usize = 8;
const PREFIX_OPTION_LEN: usize = 32;
const RDNSS_OPTION_MIN_LEN: usize = 24;
const RDNSS_ADDRESS_OFFSET: usize = 8;
const IPV6_ADDR_LEN: usize = 16;

/// Prefix information from a Router Advertisement option.
#[derive(Debug, Clone)]
pub struct PrefixInfo {
    /// IPv6 prefix.
    pub prefix: Ipv6Addr,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Whether autonomous address configuration is enabled.
    pub autonomous: bool,
    /// Valid lifetime in seconds.
    pub valid_lifetime: u32,
    /// Preferred lifetime in seconds.
    pub preferred_lifetime: u32,
}

/// Parsed Router Advertisement message.
#[derive(Debug, Clone)]
pub struct RouterAdvertisement {
    /// Hop limit value.
    pub hop_limit: u8,
    /// Managed address configuration flag.
    pub managed_flag: bool,
    /// Other configuration flag.
    pub other_flag: bool,
    /// Router lifetime in seconds.
    pub router_lifetime: u16,
    /// Source IPv6 address of the router.
    pub source: Ipv6Addr,
    /// Prefix information options.
    pub prefixes: Vec<PrefixInfo>,
    /// Recursive DNS server addresses.
    pub dns_servers: Vec<Ipv6Addr>,
    /// DNS server lifetime in seconds.
    pub dns_lifetime: u32,
}

/// `ICMPv6` parsing failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// RA packet is too short.
    #[error("RA too short: {0} bytes")]
    TooShort(usize),
    /// Packet is not a Router Advertisement.
    #[error("not a Router Advertisement: type={0}")]
    WrongType(u8),
}

/// `ICMPv6` parsing result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Constructs an `ICMPv6` Router Solicitation packet with a source link-layer address option.
#[must_use]
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

/// Parses a raw `ICMPv6` Router Advertisement into structured prefix, DNS and flag data.
///
/// # Errors
///
/// Returns [`Failure::TooShort`] when the packet is shorter than the RA header and
/// [`Failure::WrongType`] when the packet is not a Router Advertisement.
pub fn parse_router_advertisement(data: &[u8], source: Ipv6Addr) -> Result<RouterAdvertisement> {
    if data.len() < RA_HEADER_LEN {
        return Err(Failure::TooShort(data.len()));
    }

    let packet_type = read_u8(data, 0).ok_or(Failure::TooShort(data.len()))?;
    if packet_type != ICMPV6_ROUTER_ADVERTISEMENT {
        return Err(Failure::WrongType(packet_type));
    }

    let hop_limit = read_u8(data, 4).ok_or(Failure::TooShort(data.len()))?;
    let flags = read_u8(data, 5).ok_or(Failure::TooShort(data.len()))?;
    let managed_flag = (flags & 0x80) != 0;
    let other_flag = (flags & 0x40) != 0;
    let router_lifetime = read_u16(data, 6).ok_or(Failure::TooShort(data.len()))?;

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

    let Some(mut options) = data.get(RA_HEADER_LEN..) else {
        return Ok(ra);
    };

    while let Some((opt_type, option, remaining)) = split_option(options) {
        match opt_type {
            ND_OPT_PREFIX_INFO => {
                push_prefix_option(option, &mut ra.prefixes);
            }
            ND_OPT_RDNSS => parse_rdnss_option(option, &mut ra),
            _other => {}
        }

        options = remaining;
    }

    Ok(ra)
}

fn split_option(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let header = data.get(..OPTION_HEADER_LEN)?;
    let opt_type = read_u8(header, 0)?;
    let opt_len_units = usize::from(read_u8(header, 1)?);
    if opt_len_units == 0 {
        return None;
    }

    let opt_len = opt_len_units.checked_mul(OPTION_UNIT_LEN)?;
    let option = data.get(..opt_len)?;
    let remaining = data.get(opt_len..)?;

    Some((opt_type, option, remaining))
}

fn parse_prefix_option(option: &[u8]) -> Option<PrefixInfo> {
    if option.len() < PREFIX_OPTION_LEN {
        return None;
    }

    let prefix_len = read_u8(option, 2)?;
    let flags = read_u8(option, 3)?;
    let prefix = Ipv6Addr::from(read_array::<IPV6_ADDR_LEN>(option, 16)?);

    Some(PrefixInfo {
        prefix,
        prefix_len,
        autonomous: (flags & 0x40) != 0,
        valid_lifetime: read_u32(option, 4)?,
        preferred_lifetime: read_u32(option, 8)?,
    })
}

fn push_prefix_option(option: &[u8], prefixes: &mut Vec<PrefixInfo>) {
    if let Some(prefix) = parse_prefix_option(option) {
        prefixes.push(prefix);
    }
}

fn parse_rdnss_option(option: &[u8], ra: &mut RouterAdvertisement) {
    if option.len() < RDNSS_OPTION_MIN_LEN {
        return;
    }

    let Some(lifetime) = read_u32(option, 4) else {
        return;
    };
    let Some(addresses) = option.get(RDNSS_ADDRESS_OFFSET..) else {
        return;
    };

    ra.dns_lifetime = lifetime;
    for address in addresses.as_chunks::<IPV6_ADDR_LEN>().0 {
        ra.dns_servers.push(Ipv6Addr::from(*address));
    }
}

fn read_u8(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(read_array::<2>(data, offset)?))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(read_array::<4>(data, offset)?))
}

fn read_array<const N: usize>(data: &[u8], offset: usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    let slice = data.get(offset..end)?;
    let mut bytes = [0_u8; N];
    bytes.copy_from_slice(slice);
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ra_base(hop_limit: u8, flags: u8, router_lifetime: u16) -> Vec<u8> {
        let mut data = Vec::with_capacity(RA_HEADER_LEN);
        data.extend([ICMPV6_ROUTER_ADVERTISEMENT, 0, 0, 0, hop_limit, flags]);
        data.extend(router_lifetime.to_be_bytes());
        data.extend([0_u8; 8]);
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
        let flags = if autonomous { 0x40 } else { 0x00 };

        data.extend([ND_OPT_PREFIX_INFO, 4, prefix_len, flags]);
        data.extend(valid.to_be_bytes());
        data.extend(preferred.to_be_bytes());
        data.extend([0_u8; 4]);
        data.extend(prefix.octets());
    }

    fn append_rdnss_option(data: &mut Vec<u8>, lifetime: u32, servers: &[Ipv6Addr]) {
        let server_units = servers
            .len()
            .checked_mul(2)
            .expect("RDNSS server count should fit");
        let opt_len_units = 1_usize
            .checked_add(server_units)
            .expect("RDNSS option length should fit");
        let opt_len_units = u8::try_from(opt_len_units).expect("RDNSS option length should fit");

        data.extend([ND_OPT_RDNSS, opt_len_units, 0, 0]);
        data.extend(lifetime.to_be_bytes());
        for server in servers {
            data.extend(server.octets());
        }
    }

    #[test]
    fn build_router_solicitation_structure() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT
        let pkt = build_router_solicitation(&mac);

        // ASSERT
        let expected = [
            ICMPV6_ROUTER_SOLICITATION,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            ND_OPT_SOURCE_LL_ADDR,
            1,
            0xaa,
            0xbb,
            0xcc,
            0xdd,
            0xee,
            0xff,
        ];
        assert_eq!(pkt.as_slice(), expected.as_slice());
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
        parse_router_advertisement(&data, source).unwrap_err();
    }

    #[test]
    fn parse_ra_wrong_type() {
        // ARRANGE
        let mut data = make_ra_base(0, 0, 0);
        *data.first_mut().expect("RA header should have type") = ICMPV6_ROUTER_SOLICITATION;
        let source = "fe80::1".parse().expect("valid address");

        // ACT / ASSERT
        parse_router_advertisement(&data, source).unwrap_err();
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
        let parsed_prefix = ra.prefixes.first().expect("prefix should parse");
        assert_eq!(parsed_prefix.prefix, prefix);
        assert_eq!(parsed_prefix.prefix_len, 64);
        assert!(parsed_prefix.autonomous);
        assert_eq!(parsed_prefix.valid_lifetime, 7200);
        assert_eq!(parsed_prefix.preferred_lifetime, 3600);
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
        let parsed_prefix = ra.prefixes.first().expect("prefix should parse");
        assert!(!parsed_prefix.autonomous);
        assert_eq!(parsed_prefix.prefix_len, 48);
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
        assert_eq!(ra.dns_servers.as_slice(), &[dns1, dns2]);
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
        data.extend([0_u8; 30]);
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
        data.extend([0_u8; 6]);
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
