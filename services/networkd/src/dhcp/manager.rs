//! Cooperative DHCPv4 acquire loop with exponential backoff.

use std::time::Duration;

use super::DhcpLease;
use super::client::run_dhcp_client;

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(120);

/// Drives the full DORA exchange with exponential-backoff retries until a lease is acquired.
pub struct DhcpManager {
    interface: String,
    mac: [u8; 6],
    delay: Duration,
}

impl DhcpManager {
    /// Creates a new manager for the given interface.
    pub fn new(interface: String, mac: [u8; 6]) -> Self {
        Self {
            interface,
            mac,
            delay: RETRY_BASE,
        }
    }

    /// Runs DORA with backoff, resolving only once a lease is successfully acquired.
    pub async fn acquire(&mut self) -> DhcpLease {
        acquire_with_backoff(&self.interface, &self.mac, &mut self.delay).await
    }
}

async fn acquire_with_backoff(interface: &str, mac: &[u8; 6], delay: &mut Duration) -> DhcpLease {
    loop {
        match run_dhcp_client(interface, mac).await {
            Ok(lease) => return lease,
            Err(e) => {
                kmsg::warn!(
                    "DHCP failed on {}: {}; retrying in {}s",
                    interface,
                    e,
                    delay.as_secs()
                );
                tokio::time::sleep(*delay).await;
                *delay = (*delay * 2).min(RETRY_MAX);
            }
        }
    }
}
