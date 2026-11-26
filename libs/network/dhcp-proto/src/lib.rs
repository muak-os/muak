//! RFC 2131/2132 `DHCPv4` message codec.

#![warn(missing_docs)]

pub mod builder;
pub mod error;
pub(crate) mod field;
pub mod header;
pub mod message_type;
pub(crate) mod option;
pub mod options;
pub mod validate;
pub mod xid;

use core::net::Ipv4Addr;

/// Magic cookie that starts the DHCP options section.
pub const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
/// `BOOTREQUEST` value of the DHCP `op` field.
pub const BOOTREQUEST: u8 = 1;
/// Broadcast flag of the DHCP `flags` field.
pub const FLAG_BROADCAST: u16 = 0x8000;

/// UDP port a DHCP client listens on.
pub const DHCP_CLIENT_PORT: u16 = 68;
/// UDP port a DHCP server listens on.
pub const DHCP_SERVER_PORT: u16 = 67;

/// Parsed DHCP options extracted from a response.
pub struct ParsedOptions {
    /// Message type option.
    pub message_type: Option<u8>,
    /// Server identifier option.
    pub server_id: Option<Ipv4Addr>,
    /// Subnet mask option.
    pub subnet_mask: Option<Ipv4Addr>,
    /// Router option.
    pub router: Option<Ipv4Addr>,
    /// DNS server options.
    pub dns_servers: Vec<Ipv4Addr>,
    /// Lease time option, in seconds.
    pub lease_time: Option<u32>,
}
