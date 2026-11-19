//! `ICMPv6` packet construction and parsing for stateless address autoconfiguration (RFC 4861).

pub mod build;
pub mod parse;
pub mod types;

/// `ICMPv6` Router Solicitation type code.
pub const ROUTER_SOLICITATION: u8 = 133;
/// `ICMPv6` Router Advertisement type code.
pub const ROUTER_ADVERTISEMENT: u8 = 134;
