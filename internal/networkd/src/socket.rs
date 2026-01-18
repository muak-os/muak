use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd};

use anyhow::Result;

/// Wait for https://github.com/bytecodealliance/rustix/pull/1426 to be implemented in rustix
pub fn socket_bind_device<Fd: AsFd>(fd: Fd, device: &str) -> Result<()> {
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
        Err(anyhow::anyhow!(
            "Failed to bind socket to device {}: {}",
            device,
            std::io::Error::last_os_error()
        ))
    }
}
