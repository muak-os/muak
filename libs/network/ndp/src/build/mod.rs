//! Construction of `ICMPv6` Neighbor Discovery messages (RFC 4861 section 4).

pub mod router;

use core::net::Ipv6Addr;

use crate::message::NeighborFlags;
use crate::types::{self, LINK_LAYER_LEN};
use crate::{
    Failure, NEIGHBOR_ADVERTISEMENT, NEIGHBOR_SOLICITATION, ROUTER_SOLICITATION, Result, field,
};

const SOLICITATION_LEN: usize = 8;
const NEIGHBOR_LEN: usize = 24;
const LINK_LAYER_OPTION_LEN: usize = 8;
const ROUTER_FLAG: u8 = 0x80;
const SOLICITED_FLAG: u8 = 0x40;
const OVERRIDE_FLAG: u8 = 0x20;

/// Writes a Router Solicitation into `message` and returns the written bytes.
///
/// `source` and `destination` are the IPv6 addresses of the eventual datagram, used for the
/// `ICMPv6` checksum pseudo-header. `link_layer` becomes a Source Link-Layer Address option
/// only when `source` is specified, as RFC 4861 section 4.1 requires; an unspecified source
/// drops the option even when a link-layer address is supplied.
///
/// # Errors
///
/// Returns [`Failure::TooSmall`] when `message` cannot hold the solicitation.
pub fn router_solicitation(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    link_layer: Option<[u8; LINK_LAYER_LEN]>,
    message: &mut [u8],
) -> Result<&mut [u8]> {
    let link_layer = if source.is_unspecified() {
        None
    } else {
        link_layer
    };
    let required = SOLICITATION_LEN.saturating_add(link_layer.map_or(0, |_| LINK_LAYER_OPTION_LEN));
    if message.len() < required {
        return Err(Failure::TooSmall(required));
    }

    let mut cursor = field::put(message, 0, &[ROUTER_SOLICITATION, 0, 0, 0, 0, 0, 0, 0])?;
    if let Some(address) = link_layer {
        cursor = write_link_layer(message, cursor, types::SOURCE_LINK_LAYER, address)?;
    }

    field::seal(message, cursor, source, destination)
}

/// Writes a Neighbor Solicitation into `message` and returns the written bytes.
///
/// `source` and `destination` are the IPv6 addresses of the eventual datagram, used for the
/// `ICMPv6` checksum pseudo-header.
///
/// # Errors
///
/// Returns [`Failure::TooSmall`] when `message` cannot hold the solicitation.
pub fn neighbor_solicitation(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    link_layer: Option<[u8; LINK_LAYER_LEN]>,
    message: &mut [u8],
) -> Result<&mut [u8]> {
    let required = NEIGHBOR_LEN.saturating_add(link_layer.map_or(0, |_| LINK_LAYER_OPTION_LEN));
    if message.len() < required {
        return Err(Failure::TooSmall(required));
    }

    let mut cursor = field::put(message, 0, &[NEIGHBOR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0])?;
    cursor = field::put(message, cursor, &target.octets())?;
    if let Some(address) = link_layer {
        cursor = write_link_layer(message, cursor, types::SOURCE_LINK_LAYER, address)?;
    }

    field::seal(message, cursor, source, destination)
}

/// Writes a Neighbor Advertisement into `message` and returns the written bytes.
///
/// `source` and `destination` are the IPv6 addresses of the eventual datagram, used for the
/// `ICMPv6` checksum pseudo-header.
///
/// # Errors
///
/// Returns [`Failure::TooSmall`] when `message` cannot hold the advertisement.
pub fn neighbor_advertisement(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    flags: NeighborFlags,
    link_layer: Option<[u8; LINK_LAYER_LEN]>,
    message: &mut [u8],
) -> Result<&mut [u8]> {
    let required = NEIGHBOR_LEN.saturating_add(link_layer.map_or(0, |_| LINK_LAYER_OPTION_LEN));
    if message.len() < required {
        return Err(Failure::TooSmall(required));
    }

    let mut flag_bits = 0;
    if flags.router {
        flag_bits |= ROUTER_FLAG;
    }
    if flags.solicited {
        flag_bits |= SOLICITED_FLAG;
    }
    if flags.override_flag {
        flag_bits |= OVERRIDE_FLAG;
    }

    let mut cursor = field::put(
        message,
        0,
        &[NEIGHBOR_ADVERTISEMENT, 0, 0, 0, flag_bits, 0, 0, 0],
    )?;
    cursor = field::put(message, cursor, &target.octets())?;
    if let Some(address) = link_layer {
        cursor = write_link_layer(message, cursor, types::TARGET_LINK_LAYER, address)?;
    }

    field::seal(message, cursor, source, destination)
}

/// Writes a link-layer address option and returns the cursor after it.
pub(crate) fn write_link_layer(
    message: &mut [u8],
    cursor: usize,
    kind: u8,
    address: [u8; LINK_LAYER_LEN],
) -> Result<usize> {
    let cursor = field::put(message, cursor, &[kind, 1])?;

    field::put(message, cursor, &address)
}

#[cfg(test)]
mod tests {
    use core::net::Ipv6Addr;

    use super::{neighbor_advertisement, neighbor_solicitation, router_solicitation};
    use crate::message::NeighborFlags;
    use crate::types::{self, LINK_LAYER_LEN};
    use crate::{ALL_ROUTERS, Failure, checksum, parse};

    const MAC: [u8; LINK_LAYER_LEN] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

    fn source() -> Ipv6Addr {
        "fe80::1".parse().expect("valid address")
    }

    fn destination() -> Ipv6Addr {
        "fe80::2".parse().expect("valid address")
    }

    fn sum(destination: Ipv6Addr, message: &[u8]) -> u16 {
        checksum(source(), destination, message)
    }

    #[test]
    fn router_solicitation_matches_rfc_layout() {
        // ARRANGE
        let mut message = [0_u8; 16];

        // ACT
        let written = router_solicitation(source(), ALL_ROUTERS, Some(MAC), &mut message)
            .expect("buffer should hold a solicitation");

        // ASSERT
        assert_eq!(
            written,
            [
                133, 0, 0x15, 0x94, 0, 0, 0, 0, 1, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ]
        );
        assert_eq!(sum(ALL_ROUTERS, written), 0);
    }

    #[test]
    fn router_solicitation_without_link_layer_has_no_options() {
        // ARRANGE
        let mut message = [0_u8; 16];

        // ACT
        let written = router_solicitation(source(), ALL_ROUTERS, None, &mut message)
            .expect("buffer should hold a solicitation");
        let options = parse::router_solicitation(written).expect("solicitation should parse");

        // ASSERT
        assert_eq!(written.len(), 8);
        assert_eq!(options.count(), 0);
    }

    #[test]
    fn router_solicitation_unspecified_source_omits_link_layer() {
        // ARRANGE
        let mut unspecified_buffer = [0_u8; 16];
        let mut specified_buffer = [0_u8; 16];

        // ACT
        let unspecified = router_solicitation(
            Ipv6Addr::UNSPECIFIED,
            ALL_ROUTERS,
            Some(MAC),
            &mut unspecified_buffer,
        )
        .expect("buffer should hold a solicitation");
        let specified =
            router_solicitation(source(), ALL_ROUTERS, Some(MAC), &mut specified_buffer)
                .expect("buffer should hold a solicitation");

        // ASSERT
        assert_eq!(unspecified, [133, 0, 0x7b, 0xb8, 0, 0, 0, 0]);
        assert!(unspecified.len() < specified.len());
        assert_eq!(
            parse::router_solicitation(unspecified)
                .expect("solicitation should parse")
                .count(),
            0
        );
    }

    #[test]
    fn neighbor_solicitation_matches_rfc_layout() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::abcd".parse().expect("valid address");
        let mut message = [0_u8; 32];

        // ACT
        let written =
            neighbor_solicitation(source(), destination(), target, Some(MAC), &mut message)
                .expect("buffer should hold a solicitation");

        // ASSERT
        assert_eq!(
            written,
            [
                135, 0, 0x69, 0xb7, 0, 0, 0, 0, 0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0xab, 0xcd, 1, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ]
        );
    }

    #[test]
    fn neighbor_advertisement_matches_rfc_layout() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::abcd".parse().expect("valid address");
        let flags = NeighborFlags {
            router: true,
            solicited: true,
            override_flag: true,
        };
        let mut message = [0_u8; 32];

        // ACT
        let written = neighbor_advertisement(
            source(),
            destination(),
            target,
            flags,
            Some(MAC),
            &mut message,
        )
        .expect("buffer should hold an advertisement");

        // ASSERT
        assert_eq!(
            written,
            [
                136, 0, 0x87, 0xb6, 0xe0, 0, 0, 0, 0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0xab, 0xcd, 2, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ]
        );
    }

    #[test]
    fn neighbor_solicitation_roundtrips() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::abcd".parse().expect("valid address");
        let mut message = [0_u8; 40];

        // ACT
        let written =
            neighbor_solicitation(source(), destination(), target, Some(MAC), &mut message)
                .expect("buffer should hold a solicitation");
        let parsed = parse::neighbor_solicitation(written).expect("solicitation should parse");

        // ASSERT
        assert_eq!(parsed.target, target);
        assert_eq!(
            parsed.options.collect::<Vec<_>>(),
            vec![types::Option::SourceLinkLayer(MAC)]
        );
        assert_eq!(sum(destination(), written), 0);
    }

    #[test]
    fn neighbor_advertisement_roundtrips() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::abcd".parse().expect("valid address");
        let flags = NeighborFlags {
            router: true,
            solicited: true,
            override_flag: true,
        };
        let mut message = [0_u8; 40];

        // ACT
        let written = neighbor_advertisement(
            source(),
            destination(),
            target,
            flags,
            Some(MAC),
            &mut message,
        )
        .expect("buffer should hold an advertisement");
        let parsed = parse::neighbor_advertisement(written).expect("advertisement should parse");

        // ASSERT
        assert_eq!(parsed.target, target);
        assert_eq!(parsed.flags, flags);
        assert_eq!(
            parsed.options.collect::<Vec<_>>(),
            vec![types::Option::TargetLinkLayer(MAC)]
        );
        assert_eq!(sum(destination(), written), 0);
    }

    #[test]
    fn builders_reject_small_buffers() {
        // ARRANGE
        let mut solicitation_buffer = [0_u8; 4];
        let mut advertisement_buffer = [0_u8; 4];

        // ACT
        let solicitation =
            router_solicitation(source(), ALL_ROUTERS, Some(MAC), &mut solicitation_buffer);
        let advertisement = neighbor_advertisement(
            source(),
            destination(),
            Ipv6Addr::LOCALHOST,
            NeighborFlags {
                router: false,
                solicited: false,
                override_flag: false,
            },
            None,
            &mut advertisement_buffer,
        );

        // ASSERT
        assert_eq!(solicitation, Err(Failure::TooSmall(16)));
        assert_eq!(advertisement, Err(Failure::TooSmall(24)));
    }
}
