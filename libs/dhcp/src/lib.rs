//! `DHCPv4` packet codec and lease acquisition.

#![warn(missing_docs)]

pub mod connector;
pub mod exchange;
pub mod lease;
pub(crate) mod wire;

use core::result::Result as CoreResult;

use thiserror::Error;

/// Largest DHCP message the builders produce.
pub const MAX_MESSAGE_LEN: usize = 1500;
/// Largest IP frame needed to carry [`MAX_MESSAGE_LEN`].
pub const MAX_FRAME_LEN: usize = 1536;

/// `DHCPv4` exchange failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// The server sent a NAK, so the client must acquire a lease from scratch.
    #[error("DHCP server sent NAK")]
    Nak,
    /// No response arrived before the exchange deadline.
    #[error("DHCP exchange timed out")]
    Timeout,
    /// Failed to generate a random transaction ID.
    #[error("failed to generate a random DHCP transaction ID")]
    Xid,
    /// Response carries a message type the exchange did not expect.
    #[error("expected DHCP message type {expected}, got {got}")]
    MessageType {
        /// Message type the exchange expected.
        expected: u8,
        /// Message type the response carried.
        got: u8,
    },
    /// Response has no message type option.
    #[error("DHCP response missing message type option")]
    MissingMessageType,
    /// Response carries a transaction ID from another exchange.
    #[error("DHCP response transaction ID mismatch")]
    XidMismatch,
    /// Received message is shorter than the fixed header and magic cookie.
    #[error("DHCP response too short ({0} bytes)")]
    ResponseTooShort(usize),
    /// Offer carries no server identifier option.
    #[error("no server identifier in DHCPOFFER")]
    MissingServerId,
    /// IPv4 or UDP framing failed.
    #[error("IP framing failed: {0}")]
    Framing(#[from] ip::Failure),
    /// Socket input or output failed.
    #[error("DHCP socket I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Raw packet socket operation failed.
    #[error("DHCP packet socket failed: {0}")]
    Socket(#[from] packet::raw::Failure),
    /// Binding the socket to an interface failed.
    #[error("DHCP socket bind failed: {0}")]
    Bind(#[from] packet::socket::Failure),
    /// DHCP message does not fit the caller supplied buffer.
    ///
    /// Carries the required byte count.
    #[error("DHCP message needs {0} bytes, caller buffer is smaller")]
    MessageTooLong(usize),
}

/// `DHCPv4` exchange result type.
pub type Result<T> = CoreResult<T, Failure>;

/// Caller supplied buffers for one exchange: the DHCP message and the IP frame around it.
pub struct Scratch<'a> {
    /// DHCP message buffer.
    pub msg: &'a mut [u8],
    /// IP frame buffer, large enough for an IPv4 and UDP header plus [`MAX_MESSAGE_LEN`].
    pub frame: &'a mut [u8],
}

impl<'a> Scratch<'a> {
    /// Pairs a message buffer with a frame buffer.
    #[must_use = "the message and frame buffers must be used together"]
    pub const fn new(msg: &'a mut [u8], frame: &'a mut [u8]) -> Self {
        Self { msg, frame }
    }
}
