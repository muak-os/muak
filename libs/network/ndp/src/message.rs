//! Neighbor Discovery message fields (RFC 4861 section 4).

use core::net::Ipv6Addr;

use crate::types::Options;

/// Fixed fields of a Router Advertisement (RFC 4861 section 4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvertisementHeader {
    /// Default hop limit for outgoing packets.
    pub hop_limit: u8,
    /// Managed address configuration flag.
    pub managed_flag: bool,
    /// Other configuration flag.
    pub other_flag: bool,
    /// Router lifetime in seconds.
    pub router_lifetime: u16,
    /// Reachable time in milliseconds.
    pub reachable_time: u32,
    /// Retransmission timer in milliseconds.
    pub retrans_timer: u32,
}

/// Neighbor Advertisement flags (RFC 4861 section 4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeighborFlags {
    /// Sender is a router.
    pub router: bool,
    /// Advertisement answers a solicitation.
    pub solicited: bool,
    /// Advertisement overrides a cached link-layer address.
    pub override_flag: bool,
}

/// Parsed Router Advertisement (RFC 4861 section 4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouterAdvertisement<'a> {
    /// Fixed fields.
    pub header: AdvertisementHeader,
    /// Options that follow the fixed fields.
    pub options: Options<'a>,
}

/// Parsed Neighbor Solicitation (RFC 4861 section 4.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborSolicitation<'a> {
    /// Address being resolved.
    pub target: Ipv6Addr,
    /// Options that follow the fixed fields.
    pub options: Options<'a>,
}

/// Parsed Neighbor Advertisement (RFC 4861 section 4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborAdvertisement<'a> {
    /// Advertisement flags.
    pub flags: NeighborFlags,
    /// Address the advertisement is for.
    pub target: Ipv6Addr,
    /// Options that follow the fixed fields.
    pub options: Options<'a>,
}
