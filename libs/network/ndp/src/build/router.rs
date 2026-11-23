//! Construction of Router Advertisements (RFC 4861 section 4.2).

use core::net::Ipv6Addr;

use super::write_link_layer;
use crate::message::AdvertisementHeader;
use crate::types::{self, LINK_LAYER_LEN, PrefixInformation};
use crate::{Failure, ROUTER_ADVERTISEMENT, Result, field};

const ADVERTISEMENT_LEN: usize = 16;
const LINK_LAYER_OPTION_LEN: usize = 8;
const MTU_OPTION_LEN: usize = 8;
const PREFIX_OPTION_LEN: usize = 32;
const RECURSIVE_DNS_HEADER_LEN: usize = 8;
const PREFIX_OPTION_UNITS: u8 = 4;
const ADDRESS_LEN: usize = 16;
const MANAGED_FLAG: u8 = 0x80;
const OTHER_FLAG: u8 = 0x40;
const ON_LINK_FLAG: u8 = 0x80;
const AUTONOMOUS_FLAG: u8 = 0x40;

/// Router Advertisement content to write, options included.
#[derive(Clone, Copy, Debug)]
pub struct Advertisement<'a> {
    /// Fixed header fields.
    pub header: AdvertisementHeader,
    /// Source link-layer address option, written when set.
    pub link_layer: Option<[u8; LINK_LAYER_LEN]>,
    /// MTU option, written when set.
    pub mtu: Option<u32>,
    /// Prefix information options.
    pub prefixes: &'a [PrefixInformation],
    /// Recursive DNS server lifetime in seconds.
    pub dns_lifetime: u32,
    /// Recursive DNS server addresses; the builder emits a single RDNSS option when not empty.
    pub dns_servers: &'a [Ipv6Addr],
}

/// Writes a Router Advertisement into `message` and returns the written bytes.
///
/// `source` and `destination` are the IPv6 addresses of the eventual datagram, used for the
/// `ICMPv6` checksum pseudo-header.
///
/// # Errors
///
/// Returns [`Failure::TooSmall`] when `message` cannot hold the advertisement and
/// [`Failure::LengthOverflow`] when the DNS server list does not fit the option length field.
pub fn advertisement<'a>(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    advertisement: &Advertisement<'_>,
    message: &'a mut [u8],
) -> Result<&'a mut [u8]> {
    let required = advertisement_len(advertisement);
    if message.len() < required {
        return Err(Failure::TooSmall(required));
    }

    let mut flag_bits = 0;
    if advertisement.header.managed_flag {
        flag_bits |= MANAGED_FLAG;
    }
    if advertisement.header.other_flag {
        flag_bits |= OTHER_FLAG;
    }

    let mut cursor = field::put(message, 0, &[ROUTER_ADVERTISEMENT, 0, 0, 0])?;
    cursor = field::put(
        message,
        cursor,
        &[advertisement.header.hop_limit, flag_bits],
    )?;
    cursor = field::put(
        message,
        cursor,
        &advertisement.header.router_lifetime.to_be_bytes(),
    )?;
    cursor = field::put(
        message,
        cursor,
        &advertisement.header.reachable_time.to_be_bytes(),
    )?;
    cursor = field::put(
        message,
        cursor,
        &advertisement.header.retrans_timer.to_be_bytes(),
    )?;
    if let Some(address) = advertisement.link_layer {
        cursor = write_link_layer(message, cursor, types::SOURCE_LINK_LAYER, address)?;
    }
    if let Some(mtu) = advertisement.mtu {
        cursor = write_mtu(message, cursor, mtu)?;
    }
    for prefix in advertisement.prefixes {
        cursor = write_prefix_information(message, cursor, prefix)?;
    }
    if !advertisement.dns_servers.is_empty() {
        cursor = write_recursive_dns_servers(
            message,
            cursor,
            advertisement.dns_lifetime,
            advertisement.dns_servers,
        )?;
    }

    field::seal(message, cursor, source, destination)
}

const fn advertisement_len(advertisement: &Advertisement<'_>) -> usize {
    let mut len = ADVERTISEMENT_LEN;
    if advertisement.link_layer.is_some() {
        len = len.saturating_add(LINK_LAYER_OPTION_LEN);
    }
    if advertisement.mtu.is_some() {
        len = len.saturating_add(MTU_OPTION_LEN);
    }
    len = len.saturating_add(
        advertisement
            .prefixes
            .len()
            .saturating_mul(PREFIX_OPTION_LEN),
    );
    if !advertisement.dns_servers.is_empty() {
        len = len
            .saturating_add(RECURSIVE_DNS_HEADER_LEN)
            .saturating_add(advertisement.dns_servers.len().saturating_mul(ADDRESS_LEN));
    }

    len
}

fn write_mtu(message: &mut [u8], cursor: usize, mtu: u32) -> Result<usize> {
    let cursor = field::put(message, cursor, &[types::MTU, 1, 0, 0])?;

    field::put(message, cursor, &mtu.to_be_bytes())
}

fn write_prefix_information(
    message: &mut [u8],
    cursor: usize,
    prefix: &PrefixInformation,
) -> Result<usize> {
    let mut flag_bits = 0;
    if prefix.on_link {
        flag_bits |= ON_LINK_FLAG;
    }
    if prefix.autonomous {
        flag_bits |= AUTONOMOUS_FLAG;
    }

    let mut cursor = field::put(
        message,
        cursor,
        &[
            types::PREFIX_INFORMATION,
            PREFIX_OPTION_UNITS,
            prefix.prefix_len,
            flag_bits,
        ],
    )?;
    cursor = field::put(message, cursor, &prefix.valid_lifetime.to_be_bytes())?;
    cursor = field::put(message, cursor, &prefix.preferred_lifetime.to_be_bytes())?;
    cursor = field::put(message, cursor, &[0_u8; 4])?;
    field::put(message, cursor, &prefix.prefix.octets())
}

fn write_recursive_dns_servers(
    message: &mut [u8],
    cursor: usize,
    lifetime: u32,
    servers: &[Ipv6Addr],
) -> Result<usize> {
    let units = 1_usize.saturating_add(servers.len().saturating_mul(2));
    let units = u8::try_from(units).map_err(|_error| Failure::LengthOverflow(units))?;
    let mut cursor = field::put(
        message,
        cursor,
        &[types::RECURSIVE_DNS_SERVERS, units, 0, 0],
    )?;
    cursor = field::put(message, cursor, &lifetime.to_be_bytes())?;
    for server in servers {
        cursor = field::put(message, cursor, &server.octets())?;
    }

    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use core::net::Ipv6Addr;

    use super::{Advertisement, advertisement};
    use crate::message::AdvertisementHeader;
    use crate::types::{self, LINK_LAYER_LEN, PrefixInformation};
    use crate::{Failure, checksum, parse};

    const MAC: [u8; LINK_LAYER_LEN] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

    fn source() -> Ipv6Addr {
        "fe80::1".parse().expect("valid address")
    }

    fn destination() -> Ipv6Addr {
        "fe80::2".parse().expect("valid address")
    }

    fn header() -> AdvertisementHeader {
        AdvertisementHeader {
            hop_limit: 64,
            managed_flag: true,
            other_flag: false,
            router_lifetime: 1800,
            reachable_time: 4000,
            retrans_timer: 1000,
        }
    }

    fn prefix() -> PrefixInformation {
        PrefixInformation {
            prefix: "2001:db8::".parse().expect("valid prefix"),
            prefix_len: 64,
            on_link: true,
            autonomous: true,
            valid_lifetime: 7200,
            preferred_lifetime: 3600,
        }
    }

    fn sum(message: &[u8]) -> u16 {
        checksum(source(), destination(), message)
    }

    fn content<'a>(
        dns_servers: &'a [Ipv6Addr],
        prefixes: &'a [PrefixInformation],
    ) -> Advertisement<'a> {
        Advertisement {
            header: header(),
            link_layer: Some(MAC),
            mtu: Some(1500),
            prefixes,
            dns_lifetime: 3600,
            dns_servers,
        }
    }

    #[test]
    fn router_advertisement_roundtrips() {
        // ARRANGE
        let servers = [
            "2620:fe::fe".parse::<Ipv6Addr>().expect("valid address"),
            "2620:fe::9".parse::<Ipv6Addr>().expect("valid address"),
        ];
        let prefixes = [prefix()];
        let to_write = content(&servers, &prefixes);
        let mut message = [0_u8; 128];

        // ACT
        let written = advertisement(source(), destination(), &to_write, &mut message)
            .expect("buffer should hold an advertisement");
        let parsed = parse::router_advertisement(written).expect("advertisement should parse");

        // ASSERT
        assert_eq!(parsed.header, header());
        assert_eq!(sum(written), 0);
        let mut options = parsed.options;
        assert_eq!(options.next(), Some(types::Option::SourceLinkLayer(MAC)));
        assert_eq!(options.next(), Some(types::Option::Mtu(1500)));
        assert_eq!(
            options.next(),
            Some(types::Option::PrefixInformation(prefix()))
        );
        let Some(types::Option::RecursiveDnsServers(dns)) = options.next() else {
            panic!("expected recursive DNS servers");
        };
        assert_eq!(dns.lifetime, 3600);
        assert_eq!(dns.addresses.collect::<Vec<_>>(), servers.to_vec());
        assert_eq!(options.next(), None);
    }

    #[test]
    fn router_advertisement_matches_rfc_layout() {
        // ARRANGE
        let servers = [
            "2620:fe::fe".parse::<Ipv6Addr>().expect("valid address"),
            "2620:fe::9".parse::<Ipv6Addr>().expect("valid address"),
        ];
        let prefixes = [prefix()];
        let to_write = content(&servers, &prefixes);
        let mut message = [0_u8; 128];

        // ACT
        let written = advertisement(source(), destination(), &to_write, &mut message)
            .expect("buffer should hold an advertisement");
        let header = written
            .get(..16)
            .expect("advertisement starts with its header");
        let options = written
            .get(16..)
            .expect("advertisement continues with its options");

        // ASSERT
        assert_eq!(
            header,
            [
                134, 0, 0x9c, 0xca, 64, 0x80, 7, 8, 0, 0, 15, 160, 0, 0, 3, 232
            ]
        );
        assert_eq!(
            options,
            [
                1, 1, 170, 187, 204, 221, 238, 255, 5, 1, 0, 0, 0, 0, 5, 220, 3, 4, 64, 192, 0, 0,
                28, 32, 0, 0, 14, 16, 0, 0, 0, 0, 32, 1, 13, 184, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 25, 5, 0, 0, 0, 0, 14, 16, 38, 32, 0, 254, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 254,
                38, 32, 0, 254, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9
            ]
        );
    }

    #[test]
    fn router_advertisement_without_options_roundtrips() {
        // ARRANGE
        let plain = Advertisement {
            header: header(),
            link_layer: None,
            mtu: None,
            prefixes: &[],
            dns_lifetime: 0,
            dns_servers: &[],
        };
        let mut message = [0_u8; 16];

        // ACT
        let written = advertisement(source(), destination(), &plain, &mut message)
            .expect("buffer should hold an advertisement");
        let parsed = parse::router_advertisement(written).expect("advertisement should parse");

        // ASSERT
        assert_eq!(written.len(), 16);
        assert_eq!(parsed.options.count(), 0);
    }

    #[test]
    fn router_advertisement_rejects_small_buffer() {
        // ARRANGE
        let prefixes = [prefix()];
        let to_write = content(&[], &prefixes);
        let mut message = [0_u8; 4];

        // ACT
        let result = advertisement(source(), destination(), &to_write, &mut message);

        // ASSERT
        assert_eq!(result, Err(Failure::TooSmall(16 + 8 + 8 + 32)));
    }
}
