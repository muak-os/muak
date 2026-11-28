//! No std, sans-io, allocation-free RFC 4861 Neighbor Discovery message codec over RFC 4443 `ICMPv6` framing.

#![no_std]
#![warn(missing_docs)]

pub mod build;
pub mod error;
mod field;
pub mod icmpv6;
pub mod message;
pub mod parse;
pub mod types;

use core::net::Ipv6Addr;

/// IPv6 all-routers link-local multicast address.
pub const ALL_ROUTERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2);

/// IPv6 all-nodes link-local multicast address.
pub const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

/// `ICMPv6` Router Solicitation type code.
pub const ROUTER_SOLICITATION: u8 = 133;
/// `ICMPv6` Router Advertisement type code.
pub const ROUTER_ADVERTISEMENT: u8 = 134;
/// `ICMPv6` Neighbor Solicitation type code.
pub const NEIGHBOR_SOLICITATION: u8 = 135;
/// `ICMPv6` Neighbor Advertisement type code.
pub const NEIGHBOR_ADVERTISEMENT: u8 = 136;
