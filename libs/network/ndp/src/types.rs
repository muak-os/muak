//! Neighbor Discovery option types (RFC 4861 section 4.6).

use core::net::Ipv6Addr;

/// Length of the link-layer address carried by the link-layer address options.
pub const LINK_LAYER_LEN: usize = 6;

/// Source link-layer address option type.
pub(crate) const SOURCE_LINK_LAYER: u8 = 1;
/// Target link-layer address option type.
pub(crate) const TARGET_LINK_LAYER: u8 = 2;
/// Prefix information option type.
pub(crate) const PREFIX_INFORMATION: u8 = 3;
/// MTU option type.
pub(crate) const MTU: u8 = 5;
/// Recursive DNS server option type (RFC 8106 section 5.1).
pub(crate) const RECURSIVE_DNS_SERVERS: u8 = 25;

const OPTION_HEADER_LEN: usize = 2;
const OPTION_UNIT_LEN: usize = 8;
const PREFIX_INFO_LEN: usize = 32;
const PREFIX_LEN_OFFSET: usize = 2;
const PREFIX_FLAGS_OFFSET: usize = 3;
const VALID_LIFETIME_OFFSET: usize = 4;
const PREFERRED_LIFETIME_OFFSET: usize = 8;
const PREFIX_OFFSET: usize = 16;
const MTU_VALUE_OFFSET: usize = 4;
const DNS_LIFETIME_OFFSET: usize = 4;
const DNS_ADDRESSES_OFFSET: usize = 8;
const DNS_MIN_LEN: usize = 24;
const ADDRESS_LEN: usize = 16;
const ON_LINK_FLAG: u8 = 0x80;
const AUTONOMOUS_FLAG: u8 = 0x40;

/// Prefix information carried by a Router Advertisement (RFC 4861 section 4.6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixInformation {
    /// IPv6 prefix.
    pub prefix: Ipv6Addr,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Whether the prefix identifies the link.
    pub on_link: bool,
    /// Whether the prefix may be used for stateless address autoconfiguration.
    pub autonomous: bool,
    /// Valid lifetime in seconds.
    pub valid_lifetime: u32,
    /// Preferred lifetime in seconds.
    pub preferred_lifetime: u32,
}

/// Recursive DNS servers carried by a Router Advertisement (RFC 8106 section 5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecursiveDnsServers<'a> {
    /// Lifetime of the advertised servers in seconds.
    pub lifetime: u32,
    /// Advertised server addresses.
    pub addresses: DnsAddresses<'a>,
}

/// Iterator over the addresses of a recursive DNS server option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsAddresses<'a> {
    remaining: &'a [u8],
}

impl<'a> DnsAddresses<'a> {
    /// Wraps the raw address bytes of a recursive DNS server option.
    #[must_use = "the wrapped iterator is the only result"]
    pub(crate) const fn new(remaining: &'a [u8]) -> Self {
        Self { remaining }
    }
}

impl Iterator for DnsAddresses<'_> {
    type Item = Ipv6Addr;

    fn next(&mut self) -> core::option::Option<Self::Item> {
        let (octets, remaining) = self.remaining.split_first_chunk::<ADDRESS_LEN>()?;
        self.remaining = remaining;

        Some(Ipv6Addr::from(*octets))
    }
}

/// Option parsed from the options area of a Neighbor Discovery message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Option<'a> {
    /// Source link-layer address (RFC 4861 section 4.6.1).
    SourceLinkLayer([u8; LINK_LAYER_LEN]),
    /// Target link-layer address (RFC 4861 section 4.6.1).
    TargetLinkLayer([u8; LINK_LAYER_LEN]),
    /// Prefix information (RFC 4861 section 4.6.2).
    PrefixInformation(PrefixInformation),
    /// Link MTU (RFC 4861 section 4.6.4).
    Mtu(u32),
    /// Recursive DNS servers (RFC 8106 section 5.1).
    RecursiveDnsServers(RecursiveDnsServers<'a>),
}

/// Iterator over the options that follow a Neighbor Discovery message header.
///
/// Unknown option types are skipped. Iteration stops at the first option whose length does not fit the remaining bytes,
/// which also covers the zero length that RFC 4861 rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options<'a> {
    remaining: &'a [u8],
}

impl<'a> Options<'a> {
    /// Wraps the options area that follows a message header.
    #[must_use = "the wrapped iterator is the only result"]
    pub(crate) const fn new(remaining: &'a [u8]) -> Self {
        Self { remaining }
    }

    fn split_next(&mut self) -> core::option::Option<(u8, &'a [u8])> {
        let (kind, option, remaining) = split(self.remaining)?;
        self.remaining = remaining;

        Some((kind, option))
    }
}

impl<'a> Iterator for Options<'a> {
    type Item = Option<'a>;

    fn next(&mut self) -> core::option::Option<Self::Item> {
        core::iter::from_fn(|| self.split_next()).find_map(|(kind, option)| read(kind, option))
    }
}

fn split(data: &[u8]) -> core::option::Option<(u8, &[u8], &[u8])> {
    let header = data.get(..OPTION_HEADER_LEN)?;
    let kind = *header.first()?;
    let units = usize::from(*header.get(1)?);
    let len = units.checked_mul(OPTION_UNIT_LEN)?;
    if len < OPTION_HEADER_LEN {
        return None;
    }

    let option = data.get(..len)?;
    let remaining = data.get(len..)?;

    Some((kind, option, remaining))
}

fn read(kind: u8, option: &[u8]) -> core::option::Option<Option<'_>> {
    match kind {
        SOURCE_LINK_LAYER => Some(Option::SourceLinkLayer(link_layer(option)?)),
        TARGET_LINK_LAYER => Some(Option::TargetLinkLayer(link_layer(option)?)),
        PREFIX_INFORMATION => Some(Option::PrefixInformation(prefix_information(option)?)),
        MTU => Some(Option::Mtu(u32_at(option, MTU_VALUE_OFFSET)?)),
        RECURSIVE_DNS_SERVERS => Some(Option::RecursiveDnsServers(recursive_dns_servers(option)?)),
        _other => None,
    }
}

fn link_layer(option: &[u8]) -> core::option::Option<[u8; LINK_LAYER_LEN]> {
    option
        .get(OPTION_HEADER_LEN..)
        .and_then(<[u8]>::first_chunk::<LINK_LAYER_LEN>)
        .copied()
}

fn prefix_information(option: &[u8]) -> core::option::Option<PrefixInformation> {
    if option.len() < PREFIX_INFO_LEN {
        return None;
    }
    let flags = *option.get(PREFIX_FLAGS_OFFSET)?;

    Some(PrefixInformation {
        prefix: address_at(option, PREFIX_OFFSET)?,
        prefix_len: *option.get(PREFIX_LEN_OFFSET)?,
        on_link: flags & ON_LINK_FLAG != 0,
        autonomous: flags & AUTONOMOUS_FLAG != 0,
        valid_lifetime: u32_at(option, VALID_LIFETIME_OFFSET)?,
        preferred_lifetime: u32_at(option, PREFERRED_LIFETIME_OFFSET)?,
    })
}

fn recursive_dns_servers(option: &[u8]) -> core::option::Option<RecursiveDnsServers<'_>> {
    if option.len() < DNS_MIN_LEN {
        return None;
    }

    Some(RecursiveDnsServers {
        lifetime: u32_at(option, DNS_LIFETIME_OFFSET)?,
        addresses: DnsAddresses::new(option.get(DNS_ADDRESSES_OFFSET..)?),
    })
}

fn u32_at(data: &[u8], offset: usize) -> core::option::Option<u32> {
    data.get(offset..)
        .and_then(<[u8]>::first_chunk::<4>)
        .map(|bytes| u32::from_be_bytes(*bytes))
}

fn address_at(data: &[u8], offset: usize) -> core::option::Option<Ipv6Addr> {
    data.get(offset..)
        .and_then(<[u8]>::first_chunk::<ADDRESS_LEN>)
        .map(|octets| Ipv6Addr::from(*octets))
}

#[cfg(test)]
mod tests {
    use core::net::Ipv6Addr;

    use super::{Option, Options, PrefixInformation};
    use crate::types::{MTU, PREFIX_INFORMATION, SOURCE_LINK_LAYER};

    fn parse(options: &[u8]) -> Vec<Option<'_>> {
        Options::new(options).collect()
    }

    #[test]
    fn parses_source_link_layer() {
        // ARRANGE
        let options = [SOURCE_LINK_LAYER, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(
            parsed,
            vec![Option::SourceLinkLayer([
                0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ])]
        );
    }

    #[test]
    fn parses_mtu() {
        // ARRANGE
        let options = [MTU, 1, 0, 0, 0x00, 0x00, 0x05, 0xdc];

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(parsed, vec![Option::Mtu(1500)]);
    }

    #[test]
    fn parses_prefix_information() {
        // ARRANGE
        let prefix: Ipv6Addr = "2001:db8::".parse().expect("valid prefix");
        let mut options = vec![PREFIX_INFORMATION, 4, 64, 0xc0];
        options.extend(7200_u32.to_be_bytes());
        options.extend(3600_u32.to_be_bytes());
        options.extend([0_u8; 4]);
        options.extend(prefix.octets());

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(
            parsed,
            vec![Option::PrefixInformation(PrefixInformation {
                prefix,
                prefix_len: 64,
                on_link: true,
                autonomous: true,
                valid_lifetime: 7200,
                preferred_lifetime: 3600,
            })]
        );
    }

    #[test]
    fn skips_unknown_option() {
        // ARRANGE
        let mut options = vec![99, 1, 0, 0, 0, 0, 0, 0];
        options.extend([MTU, 1, 0, 0, 0, 0, 0x05, 0xdc]);

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(parsed, vec![Option::Mtu(1500)]);
    }

    #[test]
    fn stops_at_zero_length_option() {
        // ARRANGE
        let mut options = vec![PREFIX_INFORMATION, 0];
        options.extend([0_u8; 30]);

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(parsed.len(), 0);
    }

    #[test]
    fn stops_at_truncated_option() {
        // ARRANGE
        let options = [MTU, 2, 0, 0];

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(parsed.len(), 0);
    }

    #[test]
    fn stops_at_short_prefix_information() {
        // ARRANGE
        let options = [PREFIX_INFORMATION, 1, 0, 0, 0, 0, 0, 0];

        // ACT
        let parsed = parse(&options);

        // ASSERT
        assert_eq!(parsed.len(), 0);
    }
}
