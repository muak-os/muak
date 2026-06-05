//! Raw socket helpers.

use alloc::ffi::{CString, NulError};
use core::result::Result as CoreResult;
use std::io;
use std::os::fd::{AsFd, AsRawFd as _};

use thiserror::Error;

/// Socket operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Device name contains an interior null byte.
    #[error("device name contains an interior null byte: {0}")]
    InvalidDeviceName(#[from] NulError),
    /// Device name is too long for `setsockopt`.
    #[error("device name is too long for `setsockopt`: {0}")]
    InvalidDeviceNameLength(usize),
    /// Failed to bind socket to device.
    #[error("failed to bind socket to device {device}: {source}")]
    BindFailed {
        /// Device name.
        device: String,
        /// Underlying I/O error.
        source: io::Error,
    },
}

/// Socket operation result type.
pub type Result<T> = CoreResult<T, Failure>;

/// Binds a socket to a specific network device via `SO_BINDTODEVICE`.
///
/// # Errors
///
/// Returns an error when the device name contains interior NUL bytes, is too long for the socket
/// option length type, or the kernel rejects `SO_BINDTODEVICE`.
pub fn bind_device<Fd: AsFd>(fd: Fd, device: &str) -> Result<()> {
    let interface_cstr = CString::new(device)?;
    let interface_len_usize = interface_cstr.as_bytes_with_nul().len();
    let interface_len = libc::socklen_t::try_from(interface_len_usize)
        .map_err(|_len_error| Failure::InvalidDeviceNameLength(interface_len_usize))?;

    // SAFETY: We pass a valid null-terminated string and correct size, fd is valid
    let result = unsafe {
        libc::setsockopt(
            fd.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            interface_cstr.as_ptr().cast::<libc::c_void>(),
            interface_len,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(Failure::BindFailed {
            device: device.to_owned(),
            source: io::Error::last_os_error(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;

    use super::*;

    #[test]
    fn bind_device_rejects_interior_null_bytes() {
        // ARRANGE
        let socket = UdpSocket::bind("127.0.0.1:0").expect("udp socket should bind");

        // ACT
        let result = bind_device(socket, "eth\0bad");

        // ASSERT
        assert!(matches!(result, Err(Failure::InvalidDeviceName(_))));
    }
}
