//! Raw socket helpers

use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("device name contains an interior null byte: {0}")]
    InvalidDeviceName(#[from] std::ffi::NulError),
    #[error("failed to bind socket to device {device}: {source}")]
    BindFailed {
        device: String,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Binds a socket to a specific network device via `SO_BINDTODEVICE`.
pub fn bind_device<Fd: AsFd>(fd: Fd, device: &str) -> Result<()> {
    let interface_cstr = CString::new(device)?;
    // SAFETY: We pass a valid null-terminated string and correct size, fd is valid
    let result = unsafe {
        libc::setsockopt(
            fd.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            interface_cstr.as_ptr() as *const libc::c_void,
            interface_cstr.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(Error::BindFailed {
            device: device.to_string(),
            source: std::io::Error::last_os_error(),
        })
    }
}
