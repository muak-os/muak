//! `DHCPv4` option codes.

pub const SUBNET_MASK: u8 = 1;
pub const ROUTER: u8 = 3;
pub const DNS_SERVER: u8 = 6;
pub const REQUESTED_IP: u8 = 50;
pub const LEASE_TIME: u8 = 51;
pub const MESSAGE_TYPE: u8 = 53;
pub const SERVER_ID: u8 = 54;
pub const PARAM_REQUEST_LIST: u8 = 55;
pub const END: u8 = 255;
