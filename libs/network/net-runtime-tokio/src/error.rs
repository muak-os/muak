//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the tokio network runtime backend.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally ends with the module name"
)]
#[derive(Debug, Error)]
pub enum TokioError {
    /// A port operation failed at the operating system boundary.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<afpacket_linux::error::AfpacketError> for TokioError {
    fn from(error: afpacket_linux::error::AfpacketError) -> Self {
        Self::Io(error.into())
    }
}

impl From<icmpv6_linux::error::Icmpv6Error> for TokioError {
    fn from(error: icmpv6_linux::error::Icmpv6Error) -> Self {
        Self::Io(error.into())
    }
}

impl From<net_sys_linux::error::NetSysError> for TokioError {
    fn from(error: net_sys_linux::error::NetSysError) -> Self {
        Self::Io(error.into())
    }
}

impl From<TokioError> for net_runtime::error::RuntimeError {
    fn from(error: TokioError) -> Self {
        match error {
            TokioError::Io(source) => Self::Io(source),
        }
    }
}

/// Shared result type for tokio network runtime operations.
pub type Result<T> = core::result::Result<T, TokioError>;

#[cfg(test)]
mod tests {
    use std::io;

    use net_runtime::error::RuntimeError;
    use net_sys_linux::error::NetSysError;

    use super::TokioError;

    #[test]
    fn not_found_preserves_kind_and_message() {
        // ARRANGE
        let name = netmodel::Name::new("nomissing0").expect("valid name");

        // ACT
        let tokio_error = TokioError::from(NetSysError::NotFound(name));
        let tokio_message = tokio_error.to_string();
        let error = io::Error::from(RuntimeError::from(tokio_error));

        // ASSERT
        assert_eq!(tokio_message, "link nomissing0 does not exist");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "link nomissing0 does not exist");
    }

    #[test]
    fn invalid_input_leaf_conversion_keeps_the_kind() {
        // ARRANGE
        let tokio_error = TokioError::from(afpacket_linux::error::AfpacketError::InvalidDevice);

        // ACT
        let error = io::Error::from(RuntimeError::from(tokio_error));

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "device name does not fit the SO_BINDTODEVICE buffer"
        );
    }
}
