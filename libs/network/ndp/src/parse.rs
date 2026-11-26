//! Parsing of `ICMPv6` Neighbor Discovery messages (RFC 4861 section 4).

use crate::error::{NdpError, Result};
use crate::message::{
    AdvertisementHeader, NeighborAdvertisement, NeighborFlags, NeighborSolicitation,
    RouterAdvertisement,
};
use crate::types::Options;
use crate::{
    NEIGHBOR_ADVERTISEMENT, NEIGHBOR_SOLICITATION, ROUTER_ADVERTISEMENT, ROUTER_SOLICITATION, field,
};

const SOLICITATION_LEN: usize = 8;
const ADVERTISEMENT_LEN: usize = 16;
const NEIGHBOR_LEN: usize = 24;

const HOP_LIMIT: usize = 4;
const FLAGS: usize = 5;
const ROUTER_LIFETIME: usize = 6;
const REACHABLE_TIME: usize = 8;
const RETRANS_TIMER: usize = 12;
const NEIGHBOR_FLAGS: usize = 4;
const TARGET: usize = 8;

const MANAGED_FLAG: u8 = 0x80;
const OTHER_FLAG: u8 = 0x40;
const ROUTER_FLAG: u8 = 0x80;
const SOLICITED_FLAG: u8 = 0x40;
const OVERRIDE_FLAG: u8 = 0x20;

/// Parses a Router Solicitation and returns its options.
///
/// # Errors
///
/// Returns [`NdpError::Truncated`] when `message` is shorter than the solicitation header, and [`NdpError::WrongType`]
/// when the message is not a Router Solicitation.
pub fn router_solicitation(message: &[u8]) -> Result<Options<'_>> {
    check(message, ROUTER_SOLICITATION, SOLICITATION_LEN)?;
    let options = message
        .get(SOLICITATION_LEN..)
        .ok_or(NdpError::Truncated(message.len(), SOLICITATION_LEN))?;

    Ok(Options::new(options))
}

/// Parses a Router Advertisement.
///
/// # Errors
///
/// Returns [`NdpError::Truncated`] when `message` is shorter than the advertisement header, and [`NdpError::WrongType`]
/// when the message is not a Router Advertisement.
pub fn router_advertisement(message: &[u8]) -> Result<RouterAdvertisement<'_>> {
    check(message, ROUTER_ADVERTISEMENT, ADVERTISEMENT_LEN)?;
    let flags = field::byte(message, FLAGS)?;
    let header = AdvertisementHeader {
        hop_limit: field::byte(message, HOP_LIMIT)?,
        managed_flag: flags & MANAGED_FLAG != 0,
        other_flag: flags & OTHER_FLAG != 0,
        router_lifetime: field::u16_at(message, ROUTER_LIFETIME)?,
        reachable_time: field::u32_at(message, REACHABLE_TIME)?,
        retrans_timer: field::u32_at(message, RETRANS_TIMER)?,
    };
    let options = message
        .get(ADVERTISEMENT_LEN..)
        .ok_or(NdpError::Truncated(message.len(), ADVERTISEMENT_LEN))?;

    Ok(RouterAdvertisement {
        header,
        options: Options::new(options),
    })
}

/// Parses a Neighbor Solicitation.
///
/// # Errors
///
/// Returns [`NdpError::Truncated`] when `message` is shorter than the solicitation header, and [`NdpError::WrongType`]
/// when the message is not a Neighbor Solicitation.
pub fn neighbor_solicitation(message: &[u8]) -> Result<NeighborSolicitation<'_>> {
    check(message, NEIGHBOR_SOLICITATION, NEIGHBOR_LEN)?;

    Ok(NeighborSolicitation {
        target: field::address_at(message, TARGET)?,
        options: Options::new(options(message)?),
    })
}

/// Parses a Neighbor Advertisement.
///
/// # Errors
///
/// Returns [`NdpError::Truncated`] when `message` is shorter than the advertisement header, and [`NdpError::WrongType`]
/// when the message is not a Neighbor Advertisement.
pub fn neighbor_advertisement(message: &[u8]) -> Result<NeighborAdvertisement<'_>> {
    check(message, NEIGHBOR_ADVERTISEMENT, NEIGHBOR_LEN)?;
    let flags = field::byte(message, NEIGHBOR_FLAGS)?;

    Ok(NeighborAdvertisement {
        flags: NeighborFlags {
            router: flags & ROUTER_FLAG != 0,
            solicited: flags & SOLICITED_FLAG != 0,
            override_flag: flags & OVERRIDE_FLAG != 0,
        },
        target: field::address_at(message, TARGET)?,
        options: Options::new(options(message)?),
    })
}

fn check(message: &[u8], kind: u8, minimum: usize) -> Result<()> {
    if message.len() < minimum {
        return Err(NdpError::Truncated(message.len(), minimum));
    }
    let actual = field::byte(message, 0)?;
    if actual != kind {
        return Err(NdpError::WrongType(actual));
    }

    Ok(())
}

fn options(message: &[u8]) -> Result<&[u8]> {
    message
        .get(NEIGHBOR_LEN..)
        .ok_or(NdpError::Truncated(message.len(), NEIGHBOR_LEN))
}

#[cfg(test)]
mod tests {
    use core::net::Ipv6Addr;

    use super::{neighbor_advertisement, neighbor_solicitation, router_advertisement};
    use crate::error::NdpError;
    use crate::types::Option;
    use crate::{
        NEIGHBOR_ADVERTISEMENT, NEIGHBOR_SOLICITATION, ROUTER_ADVERTISEMENT, ROUTER_SOLICITATION,
    };

    fn ra_base(hop_limit: u8, flags: u8, router_lifetime: u16) -> Vec<u8> {
        let mut message = vec![ROUTER_ADVERTISEMENT, 0, 0, 0, hop_limit, flags];
        message.extend(router_lifetime.to_be_bytes());
        message.extend(4000_u32.to_be_bytes());
        message.extend(1000_u32.to_be_bytes());

        message
    }

    fn ns_base(target: Ipv6Addr) -> Vec<u8> {
        let mut message = vec![NEIGHBOR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];
        message.extend(target.octets());

        message
    }

    fn na_base(flags: u8, target: Ipv6Addr) -> Vec<u8> {
        let mut message = vec![NEIGHBOR_ADVERTISEMENT, 0, 0, 0, flags, 0, 0, 0];
        message.extend(target.octets());

        message
    }

    #[test]
    fn parses_router_advertisement_header() {
        // ARRANGE
        let message = ra_base(64, 0xc0, 1800);

        // ACT
        let parsed = router_advertisement(&message).expect("advertisement should parse");

        // ASSERT
        assert_eq!(parsed.header.hop_limit, 64);
        assert!(parsed.header.managed_flag);
        assert!(parsed.header.other_flag);
        assert_eq!(parsed.header.router_lifetime, 1800);
        assert_eq!(parsed.header.reachable_time, 4000);
        assert_eq!(parsed.header.retrans_timer, 1000);
        assert_eq!(parsed.options.count(), 0);
    }

    #[test]
    fn parses_router_advertisement_without_flags() {
        // ARRANGE
        let message = ra_base(0, 0x00, 0);

        // ACT
        let parsed = router_advertisement(&message).expect("advertisement should parse");

        // ASSERT
        assert!(!parsed.header.managed_flag);
        assert!(!parsed.header.other_flag);
    }

    #[test]
    fn rejects_short_router_advertisement() {
        // ARRANGE
        let message = vec![ROUTER_ADVERTISEMENT, 0, 0, 0];

        // ACT
        let result = router_advertisement(&message);

        // ASSERT
        assert_eq!(result, Err(NdpError::Truncated(4, 16)));
    }

    #[test]
    fn rejects_wrong_router_advertisement_type() {
        // ARRANGE
        let mut message = ra_base(64, 0, 1800);
        message[0] = ROUTER_SOLICITATION;

        // ACT
        let result = router_advertisement(&message);

        // ASSERT
        assert_eq!(result, Err(NdpError::WrongType(ROUTER_SOLICITATION)));
    }

    #[test]
    fn parses_router_solicitation_options() {
        // ARRANGE
        let mut message = vec![ROUTER_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];
        message.extend([1, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);

        // ACT
        let options = super::router_solicitation(&message).expect("solicitation should parse");

        // ASSERT
        assert_eq!(
            options.collect::<Vec<_>>(),
            vec![Option::SourceLinkLayer([
                0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ])]
        );
    }

    #[test]
    fn parses_neighbor_solicitation_target() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::1".parse().expect("valid address");
        let message = ns_base(target);

        // ACT
        let parsed = neighbor_solicitation(&message).expect("solicitation should parse");

        // ASSERT
        assert_eq!(parsed.target, target);
        assert_eq!(parsed.options.count(), 0);
    }

    #[test]
    fn rejects_short_neighbor_solicitation() {
        // ARRANGE
        let message = ns_base(Ipv6Addr::LOCALHOST);

        // ACT
        let result = neighbor_solicitation(&message[..16]);

        // ASSERT
        assert_eq!(result, Err(NdpError::Truncated(16, 24)));
    }

    #[test]
    fn parses_neighbor_advertisement_flags_and_target() {
        // ARRANGE
        let target: Ipv6Addr = "fe80::2".parse().expect("valid address");
        let message = na_base(0xe0, target);

        // ACT
        let parsed = neighbor_advertisement(&message).expect("advertisement should parse");

        // ASSERT
        assert!(parsed.flags.router);
        assert!(parsed.flags.solicited);
        assert!(parsed.flags.override_flag);
        assert_eq!(parsed.target, target);
    }

    #[test]
    fn parses_neighbor_advertisement_without_flags() {
        // ARRANGE
        let message = na_base(0x00, Ipv6Addr::LOCALHOST);

        // ACT
        let parsed = neighbor_advertisement(&message).expect("advertisement should parse");

        // ASSERT
        assert!(!parsed.flags.router);
        assert!(!parsed.flags.solicited);
        assert!(!parsed.flags.override_flag);
    }

    #[test]
    fn rejects_short_neighbor_advertisement() {
        // ARRANGE
        let message = na_base(0, Ipv6Addr::LOCALHOST);

        // ACT
        let result = neighbor_advertisement(&message[..20]);

        // ASSERT
        assert_eq!(result, Err(NdpError::Truncated(20, 24)));
    }
}
