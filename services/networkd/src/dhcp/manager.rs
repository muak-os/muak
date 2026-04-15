//! Cooperative DHCPv4 acquire loop with exponential backoff.

use std::time::Duration;

use anyhow::Result;
use tokio::net::UdpSocket;

use super::DhcpLease;
use super::client::{DhcpConnector, run_dhcp_client};

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(120);

/// Drives the full DORA exchange with exponential-backoff retries until a lease is acquired.
pub struct DhcpManager {
    socket: UdpSocket,
    mac: [u8; 6],
    delay: Duration,
}

impl DhcpManager {
    /// Creates a new manager to bind a reusable socket for the given interface.
    pub async fn new<C: DhcpConnector>(
        interface: &str,
        mac: [u8; 6],
        connector: &C,
    ) -> Result<Self> {
        let socket = connector.create(interface).await?;
        Ok(Self {
            socket,
            mac,
            delay: RETRY_BASE,
        })
    }

    /// Returns a reference to the underlying socket for reuse in renew/rebind operations.
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    /// Runs DORA with backoff, resolving only once a lease is successfully acquired.
    pub async fn acquire(&mut self) -> DhcpLease {
        acquire_with_backoff(&self.socket, &self.mac, &mut self.delay).await
    }
}

async fn acquire_with_backoff(
    socket: &UdpSocket,
    mac: &[u8; 6],
    delay: &mut Duration,
) -> DhcpLease {
    loop {
        match run_dhcp_client(socket, mac).await {
            Ok(lease) => return lease,
            Err(e) => {
                kmsg::warn!("DHCP failed: {}; retrying in {}s", e, delay.as_secs());
                tokio::time::sleep(*delay).await;
                *delay = (*delay * 2).min(RETRY_MAX);
            }
        }
    }
}
