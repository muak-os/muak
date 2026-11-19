//! `DHCPv4` wire format: message construction and response parsing (RFC 2131).

pub(crate) mod builder;
pub(crate) mod field;
pub(crate) mod header;
pub(crate) mod message_type;
pub(crate) mod option;
pub(crate) mod options;
pub(crate) mod validate;
pub(crate) mod xid;

/// Magic cookie that starts the DHCP options section.
pub(crate) const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
/// `BOOTREQUEST` value of the DHCP `op` field.
pub(crate) const BOOTREQUEST: u8 = 1;
/// Broadcast flag of the DHCP `flags` field.
pub(crate) const FLAG_BROADCAST: u16 = 0x8000;

/// UDP port a DHCP client listens on.
pub(crate) const DHCP_CLIENT_PORT: u16 = 68;
/// UDP port a DHCP server listens on.
pub(crate) const DHCP_SERVER_PORT: u16 = 67;

/// Seconds to wait for a server response.
pub(crate) const TIMEOUT_SECS: u64 = 10;
/// Lease duration used when the server omits the option.
pub(crate) const DEFAULT_LEASE_SECS: u32 = 3600;
/// Prefix length used when the server omits the subnet mask.
pub(crate) const DEFAULT_PREFIX_LEN: u8 = 24;
