//! `ICMPv6` packet construction and parsing for stateless address autoconfiguration (RFC 4861).

pub mod build;
pub mod parse;
pub mod types;

use core::net::Ipv6Addr;

/// IPv6 all-routers link-local multicast address.
pub const ALL_ROUTERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);

/// `ICMPv6` Router Solicitation type code.
pub const ROUTER_SOLICITATION: u8 = 133;
/// `ICMPv6` Router Advertisement type code.
pub const ROUTER_ADVERTISEMENT: u8 = 134;
