//! Safe wrapper for acquiring the pre-bound socket from granola.

use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;

use anyhow::{Context, Result};
use tokio::net::UnixListener;

/// FD number where granola places the pre-bound service socket.
const GRANOLA_SOCKET_FD: i32 = 3;

/// Acquires the pre-bound UNIX socket.
pub fn socket() -> Result<UnixListener> {
    // SAFETY: granola pre-binds the socket before exec.
    // Each service calls this at most once, so no duplicate ownership.
    let std_listener = unsafe { StdUnixListener::from_raw_fd(GRANOLA_SOCKET_FD) };
    std_listener
        .set_nonblocking(true)
        .context("failed to set granola socket non-blocking")?;
    UnixListener::from_std(std_listener).context("failed to create async UnixListener")
}
