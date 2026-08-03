//! Cooperative `DHCPv4` acquire loop with exponential backoff.

use core::time::Duration;

use anyhow::Result;
use netlib::packet::Socket;
use tokio::time::sleep;

use super::Lease;
use super::client::{self, DhcpConnector};

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_mins(2);

/// Drives the full DORA exchange with exponential-backoff retries until a lease is acquired.
pub struct Manager {
    socket: Socket,
    mac: [u8; 6],
    delay: Duration,
}

impl Manager {
    /// Creates a new manager binding a raw packet socket for the given interface.
    ///
    /// # Errors
    ///
    /// Returns an error if the raw packet socket cannot be opened.
    pub async fn new<C: DhcpConnector>(
        interface: &str,
        mac: [u8; 6],
        connector: &C,
    ) -> Result<Self> {
        let socket = connector.create_raw(interface).await?;
        Ok(Self {
            socket,
            mac,
            delay: RETRY_BASE,
        })
    }

    /// Runs DORA with backoff, resolving only once a lease is successfully acquired.
    pub async fn acquire(&mut self) -> Lease {
        loop {
            match client::run(&self.socket, &self.mac).await {
                Ok(lease) => return lease,
                Err(e) => kmsg::warn!("DHCP failed: {}; retrying in {}s", e, self.delay.as_secs()),
            }
            sleep(self.delay).await;
            self.delay = self.delay.saturating_mul(2).min(RETRY_MAX);
        }
    }

    /// Returns a reference to the underlying raw socket for reuse in rebind operations.
    #[must_use]
    pub fn socket(&self) -> &Socket {
        &self.socket
    }
}
