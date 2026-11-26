//! Error type and shared result alias.

use thiserror::Error;

/// Errors returned by the `DHCPv4` message codec.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum DhcpProtoError {
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

/// Shared result type for `DHCPv4` codec operations.
pub type Result<T> = core::result::Result<T, DhcpProtoError>;
