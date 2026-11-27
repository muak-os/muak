//! Error type and shared result alias.

use std::io;

use thiserror::Error;

/// Errors reported by the Linux network primitives.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Debug, Error)]
pub enum NetSysError {
    /// The named link does not exist in the kernel.
    #[error("link {0} does not exist")]
    NotFound(netmodel::Name),
    /// An operating system call failed.
    #[error("net sys operation failed: {0}")]
    Io(#[from] io::Error),
}

impl From<NetSysError> for io::Error {
    fn from(error: NetSysError) -> Self {
        match error {
            NetSysError::NotFound(name) => io::Error::new(
                io::ErrorKind::NotFound,
                format!("link {name} does not exist"),
            ),
            NetSysError::Io(source) => source,
        }
    }
}

/// Shared result type for Linux network primitive operations.
pub type Result<T> = core::result::Result<T, NetSysError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_converts_to_a_not_found_error() {
        // ARRANGE
        let name = netmodel::Name::new("nomissing0").expect("valid name");

        // ACT
        let error = io::Error::from(NetSysError::NotFound(name));

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "link nomissing0 does not exist");
    }

    #[test]
    fn io_conversion_keeps_the_source_kind() {
        // ARRANGE
        let error = NetSysError::Io(io::Error::from_raw_os_error(libc::EPERM));

        // ACT
        let error = io::Error::from(error);

        // ASSERT
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
