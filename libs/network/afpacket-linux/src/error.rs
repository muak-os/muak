//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the `AF_PACKET` socket adapter.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum AfpacketError {
    /// The device name is empty or does not fit the `SO_BINDTODEVICE` buffer.
    #[error("device name does not fit the SO_BINDTODEVICE buffer")]
    InvalidDevice,
    /// The device name length does not fit the `socklen_t` range.
    #[error("device name length exceeds socklen_t range")]
    InvalidDeviceLength,
    /// The interface index does not fit the `sockaddr_ll` index field.
    #[error("interface index exceeds i32 range")]
    IndexOverflow,
    /// The descriptor is not a bound `AF_PACKET` socket.
    #[error("socket is not bound to an AF_PACKET device")]
    NotBound,
    /// Socket creation, option or data path operations failed.
    #[error("packet socket operation failed: {0}")]
    Io(#[from] io::Error),
}

impl From<AfpacketError> for io::Error {
    fn from(error: AfpacketError) -> Self {
        match error {
            AfpacketError::InvalidDevice => io::Error::new(
                io::ErrorKind::InvalidInput,
                "device name does not fit the SO_BINDTODEVICE buffer",
            ),
            AfpacketError::InvalidDeviceLength => io::Error::new(
                io::ErrorKind::InvalidInput,
                "device name length exceeds socklen_t range",
            ),
            AfpacketError::IndexOverflow => io::Error::new(
                io::ErrorKind::InvalidData,
                "interface index exceeds i32 range",
            ),
            AfpacketError::NotBound => io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket is not bound to an AF_PACKET device",
            ),
            AfpacketError::Io(source) => source,
        }
    }
}

/// Shared result type for `AF_PACKET` socket adapter operations.
pub type Result<T> = core::result::Result<T, AfpacketError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_variants_convert_to_their_io_kinds() {
        // ARRANGE
        let invalid_device = AfpacketError::InvalidDevice;
        let invalid_device_length = AfpacketError::InvalidDeviceLength;
        let index_overflow = AfpacketError::IndexOverflow;
        let not_bound = AfpacketError::NotBound;

        // ACT
        let invalid_device = io::Error::from(invalid_device);
        let invalid_device_length = io::Error::from(invalid_device_length);
        let index_overflow = io::Error::from(index_overflow);
        let not_bound = io::Error::from(not_bound);

        // ASSERT
        assert_eq!(invalid_device.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            invalid_device.to_string(),
            "device name does not fit the SO_BINDTODEVICE buffer"
        );
        assert_eq!(invalid_device_length.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            invalid_device_length.to_string(),
            "device name length exceeds socklen_t range"
        );
        assert_eq!(index_overflow.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            index_overflow.to_string(),
            "interface index exceeds i32 range"
        );
        assert_eq!(not_bound.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            not_bound.to_string(),
            "socket is not bound to an AF_PACKET device"
        );
    }

    #[test]
    fn io_conversion_keeps_the_source_kind() {
        // ARRANGE
        let error = AfpacketError::Io(io::Error::from_raw_os_error(libc::EPERM));

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
