use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustix::net::{
    AddressFamily, SocketAddrUnix, SocketFlags, SocketType, bind, listen, socket_with,
};

use crate::supervisor::SERVICES_DIR;

/// Returns the canonical socket path for a named service.
pub fn path(name: &str) -> PathBuf {
    PathBuf::from(format!("{SERVICES_DIR}/{name}.sock"))
}

/// Pre-binds and listens on a UNIX stream socket.
pub fn pre_bind(path: &Path) -> Result<OwnedFd> {
    let _ = std::fs::remove_file(path);

    let fd = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .context("Failed to create socket")?;

    let addr = SocketAddrUnix::new(path).context("Failed to build socket address")?;
    bind(&fd, &addr).context("Failed to bind socket")?;
    listen(&fd, 128).context("Failed to listen on socket")?;

    Ok(fd)
}
