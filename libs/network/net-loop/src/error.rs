//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the driver loop.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally ends with the module name"
)]
#[derive(Debug, Error)]
pub enum LoopError {
    /// The controller table has no entry for the interface the input belongs to.
    #[error("no interface controller for the input")]
    UnknownInterface,
    /// The socket specification names an interface other than the effect owner.
    #[error("socket spec link does not match the owning interface")]
    SocketSpecLink,
    /// The socket table holds no entry for the requested slot.
    #[error("no socket open for the requested slot")]
    SocketMissing,
    /// The socket table entry vanished between the lookup and the dispatch.
    #[error("socket table entry disappeared during dispatch")]
    SocketTableMissing,
    /// An `ICMPv6` send was requested without an IPv6 destination.
    #[error("ICMPv6 send requires an IPv6 destination")]
    MissingIcmpv6Destination,
    /// A destination was passed for a socket kind that carries it in the frame.
    #[error("only an ICMPv6 socket takes a destination address")]
    UnexpectedDestination,
    /// The bridge link could not be read back after the runtime created it.
    #[error("bridge link is missing after creation")]
    MissingBridge,
    /// A runtime port reported a failure.
    #[error(transparent)]
    Runtime(#[from] net_runtime::error::RuntimeError),
}

impl From<LoopError> for io::Error {
    fn from(error: LoopError) -> Self {
        match error {
            LoopError::UnknownInterface => Self::new(
                io::ErrorKind::NotFound,
                "no interface controller for the input",
            ),
            LoopError::SocketSpecLink => Self::new(
                io::ErrorKind::InvalidInput,
                "socket spec link does not match the owning interface",
            ),
            LoopError::SocketMissing => Self::new(
                io::ErrorKind::NotFound,
                "no socket open for the requested slot",
            ),
            LoopError::SocketTableMissing => Self::new(
                io::ErrorKind::NotFound,
                "socket table entry disappeared during dispatch",
            ),
            LoopError::MissingIcmpv6Destination => Self::new(
                io::ErrorKind::InvalidInput,
                "ICMPv6 send requires an IPv6 destination",
            ),
            LoopError::UnexpectedDestination => Self::new(
                io::ErrorKind::InvalidInput,
                "only an ICMPv6 socket takes a destination address",
            ),
            LoopError::MissingBridge => Self::new(
                io::ErrorKind::NotFound,
                "bridge link is missing after creation",
            ),
            LoopError::Runtime(source) => Self::from(source),
        }
    }
}

/// Shared result type for driver loop operations.
pub type Result<T> = core::result::Result<T, LoopError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_variants_convert_to_their_former_io_kinds() {
        // ARRANGE
        let failures = [
            (LoopError::UnknownInterface, io::ErrorKind::NotFound),
            (LoopError::SocketSpecLink, io::ErrorKind::InvalidInput),
            (LoopError::SocketMissing, io::ErrorKind::NotFound),
            (LoopError::SocketTableMissing, io::ErrorKind::NotFound),
            (
                LoopError::MissingIcmpv6Destination,
                io::ErrorKind::InvalidInput,
            ),
            (
                LoopError::UnexpectedDestination,
                io::ErrorKind::InvalidInput,
            ),
            (LoopError::MissingBridge, io::ErrorKind::NotFound),
        ];

        for (error, expected) in failures {
            // ACT
            let message = error.to_string();
            let converted = io::Error::from(error);

            // ASSERT
            assert_eq!(converted.kind(), expected);
            assert_eq!(converted.to_string(), message);
        }
    }

    #[test]
    fn runtime_failure_keeps_the_io_kind_and_message() {
        // ARRANGE
        let source = io::Error::new(io::ErrorKind::TimedOut, "port timed out");
        let error = LoopError::Runtime(net_runtime::error::RuntimeError::Io(source));

        // ACT
        let converted = io::Error::from(error);

        // ASSERT
        assert_eq!(converted.kind(), io::ErrorKind::TimedOut);
        assert_eq!(converted.to_string(), "port timed out");
    }
}
