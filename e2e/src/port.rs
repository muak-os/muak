use std::net::TcpListener;

use anyhow::{Context, Result};

/// Allocates a free TCP port by binding to port 0 and reading the assigned port.
pub fn allocate() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind ephemeral TCP port")?;
    let port = listener
        .local_addr()
        .context("failed to read local address")?
        .port();
    drop(listener);
    Ok(port)
}
