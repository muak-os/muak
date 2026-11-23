//! Error type for `DHCPv4` client operations.

use thiserror::Error;

/// Error type for `DHCPv4` client operations.
#[derive(Debug, Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "DhcpError is the canonical error type"
)]
pub enum DhcpError {
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

impl From<dhcp_proto::Failure> for DhcpError {
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

/// Result type alias for `DHCPv4` client operations.
pub type Result<T> = core::result::Result<T, DhcpError>;
