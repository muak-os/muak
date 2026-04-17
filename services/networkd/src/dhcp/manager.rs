//! Cooperative DHCPv4 acquire loop with exponential backoff.

use std::time::Duration;

use anyhow::Result;
use netlib::packet::PacketSocket;

use super::DhcpLease;
use super::client::{self, DhcpConnector};

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(120);

/// Drives the full DORA exchange with exponential-backoff retries until a lease is acquired.
pub struct DhcpManager {
    socket: PacketSocket,
    mac: [u8; 6],
    delay: Duration,
}

impl DhcpManager {
    /// Creates a new manager binding a raw packet socket for the given interface.
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

    /// Returns a reference to the underlying raw socket for reuse in rebind operations.
    pub fn socket(&self) -> &PacketSocket {
        &self.socket
    }

    /// Runs DORA with backoff, resolving only once a lease is successfully acquired.
    pub async fn acquire(&mut self) -> DhcpLease {
        loop {
            match client::run(&self.socket, &self.mac).await {
                Ok(lease) => return lease,
                Err(e) => kmsg::warn!("DHCP failed: {}; retrying in {}s", e, self.delay.as_secs()),
            }
            tokio::time::sleep(self.delay).await;
            self.delay = (self.delay * 2).min(RETRY_MAX);
        }
    }
}
