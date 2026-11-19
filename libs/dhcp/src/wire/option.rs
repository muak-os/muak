//! `DHCPv4` option codes.

pub(crate) const SUBNET_MASK: u8 = 1;
pub(crate) const ROUTER: u8 = 3;
pub(crate) const DNS_SERVER: u8 = 6;
pub(crate) const REQUESTED_IP: u8 = 50;
pub(crate) const LEASE_TIME: u8 = 51;
pub(crate) const MESSAGE_TYPE: u8 = 53;
pub(crate) const SERVER_ID: u8 = 54;
pub(crate) const PARAM_REQUEST_LIST: u8 = 55;
pub(crate) const END: u8 = 255;
