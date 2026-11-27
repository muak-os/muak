//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the raw `ICMPv6` socket adapter.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum Icmpv6Error {
    /// The device name is empty or does not fit the `SO_BINDTODEVICE` buffer.
    #[error("device name does not fit the SO_BINDTODEVICE buffer")]
    InvalidDevice,
    /// The device name length does not fit the `socklen_t` range.
    #[error("device name length exceeds socklen_t range")]
    InvalidDeviceLength,
    /// The `ICMP6_FILTER` option length does not fit the `socklen_t` range.
    #[error("ICMP6_FILTER length exceeds socklen_t range")]
    InvalidFilter,
    /// Socket creation, option or data path operations failed.
    #[error("icmpv6 socket operation failed: {0}")]
    Io(#[from] io::Error),
}

impl From<Icmpv6Error> for io::Error {
    fn from(error: Icmpv6Error) -> Self {
        match error {
            Icmpv6Error::InvalidDevice => io::Error::new(
                io::ErrorKind::InvalidInput,
                "device name does not fit the SO_BINDTODEVICE buffer",
            ),
            Icmpv6Error::InvalidDeviceLength => io::Error::new(
                io::ErrorKind::InvalidInput,
                "device name length exceeds socklen_t range",
            ),
            Icmpv6Error::InvalidFilter => io::Error::new(
                io::ErrorKind::InvalidInput,
                "ICMP6_FILTER length exceeds socklen_t range",
            ),
            Icmpv6Error::Io(source) => source,
        }
    }
}

/// Shared result type for raw `ICMPv6` socket adapter operations.
pub type Result<T> = core::result::Result<T, Icmpv6Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_variants_convert_to_invalid_input() {
        // ARRANGE
        let invalid_device = Icmpv6Error::InvalidDevice;
        let invalid_device_length = Icmpv6Error::InvalidDeviceLength;
        let invalid_filter = Icmpv6Error::InvalidFilter;

        // ACT
        let invalid_device = io::Error::from(invalid_device);
        let invalid_device_length = io::Error::from(invalid_device_length);
        let invalid_filter = io::Error::from(invalid_filter);

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
        assert_eq!(invalid_filter.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            invalid_filter.to_string(),
            "ICMP6_FILTER length exceeds socklen_t range"
        );
    }

    #[test]
    fn io_conversion_keeps_the_source_kind() {
        // ARRANGE
        let error = Icmpv6Error::Io(io::Error::from_raw_os_error(libc::EPERM));

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
