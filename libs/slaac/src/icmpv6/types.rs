//! Types describing a parsed `ICMPv6` Router Advertisement.

use core::net::Ipv6Addr;

use thiserror::Error;

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
