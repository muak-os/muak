//! `DHCPv4` message type values.

/// `DHCPOFFER` message type.
pub const OFFER: u8 = 2;
/// `DHCPACK` message type.
pub const ACK: u8 = 5;

pub(crate) const DISCOVER: u8 = 1;
pub(crate) const REQUEST: u8 = 3;
pub(crate) const NAK: u8 = 6;
