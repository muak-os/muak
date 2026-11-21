//! `DHCPv4` lease acquisition client.

#![warn(missing_docs)]

pub mod datagram;
pub mod exchange;
pub mod lease;

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
    /// Offer carries no server identifier option.
    #[error("no server identifier in DHCPOFFER")]
    MissingServerId,
    /// The response carried no usable address and no fallback address was available.
    #[error("DHCP response carried no usable address")]
    MissingAddress,
    /// Message construction or response validation failed.
    #[error("{0}")]
    Proto(#[source] dhcp_proto::Failure),
    /// IPv4 or UDP framing failed.
    #[error("IP framing failed: {0}")]
    Framing(#[from] ip::Failure),
    /// Socket input or output failed.
    #[error("DHCP socket I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Raw packet socket operation failed.
    #[error("DHCP packet socket failed: {0}")]
    Socket(#[from] packet::v4::Failure),
    /// Binding the socket to an interface failed.
    #[error("DHCP socket bind failed: {0}")]
    Bind(#[from] packet::socket::Failure),
}

impl From<dhcp_proto::Failure> for Failure {
    fn from(failure: dhcp_proto::Failure) -> Self {
        match failure {
            dhcp_proto::Failure::Nak => Self::Nak,
            other @ (dhcp_proto::Failure::Xid
            | dhcp_proto::Failure::MessageTooLong { .. }
            | dhcp_proto::Failure::ResponseTooShort(_)
            | dhcp_proto::Failure::XidMismatch
            | dhcp_proto::Failure::MissingMessageType
            | dhcp_proto::Failure::MessageType { .. }) => Self::Proto(other),
        }
    }
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
