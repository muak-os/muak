//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the TAP adapter.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum TapError {
    /// The device name is empty or does not fit the kernel `IFNAMSIZ` limit.
    #[error("tap name must be non empty and shorter than IFNAMSIZ")]
    InvalidName,
    /// Opening the tun device or issuing an ioctl on it failed.
    #[error("tap device operation failed: {0}")]
    Io(#[from] io::Error),
}

impl From<TapError> for io::Error {
    fn from(error: TapError) -> Self {
        match error {
            TapError::InvalidName => io::Error::new(
                io::ErrorKind::InvalidInput,
                "tap name must be non empty and shorter than IFNAMSIZ",
            ),
            TapError::Io(source) => source,
        }
    }
}

/// Shared result type for TAP adapter operations.
pub type Result<T> = core::result::Result<T, TapError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_name_converts_to_invalid_input() {
        // ARRANGE
        let error = TapError::InvalidName;

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "tap name must be non empty and shorter than IFNAMSIZ"
        );
    }

    #[test]
    fn io_conversion_keeps_the_source_kind() {
        // ARRANGE
        let error = TapError::Io(io::Error::from_raw_os_error(libc::EBUSY));

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
    }
}
