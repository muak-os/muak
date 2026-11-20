//! `DHCPv4` message codec (RFC 2131).

#![warn(missing_docs)]

pub mod builder;
pub(crate) mod field;
pub mod header;
pub mod message_type;
pub(crate) mod option;
pub mod options;
pub mod validate;
pub mod xid;

use core::net::Ipv4Addr;
use core::result::Result as CoreResult;

use thiserror::Error;

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

/// `DHCPv4` codec failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to generate a random transaction ID.
    #[error("failed to generate a random DHCP transaction ID")]
    Xid,
    /// DHCP message does not fit the caller supplied buffer.
    #[error("DHCP message needs {required} bytes, buffer holds {capacity}")]
    MessageTooLong {
        /// Bytes the message requires.
        required: usize,
        /// Bytes the caller supplied.
        capacity: usize,
    },
    /// Received message is shorter than the fixed header and magic cookie.
    #[error("DHCP response too short ({0} bytes)")]
    ResponseTooShort(usize),
    /// Response carries a transaction ID from another exchange.
    #[error("DHCP response transaction ID mismatch")]
    XidMismatch,
    /// Response has no message type option.
    #[error("DHCP response missing message type option")]
    MissingMessageType,
    /// Response carries a message type the exchange did not expect.
    #[error("expected DHCP message type {expected}, got {got}")]
    MessageType {
        /// Message type the exchange expected.
        expected: u8,
        /// Message type the response carried.
        got: u8,
    },
    /// The server sent a NAK, so the client must acquire a lease from scratch.
    #[error("DHCP server sent NAK")]
    Nak,
}

/// `DHCPv4` codec result type.
pub type Result<T> = CoreResult<T, Failure>;

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
