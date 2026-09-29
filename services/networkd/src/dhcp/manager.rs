//! Cooperative `DHCPv4` acquire loop with exponential backoff.

use core::future::Future;
use core::time::Duration;

use anyhow::Result;
use netlib::packet::Socket;
use tokio::time::sleep;

use super::Lease;
use super::client::{self, DhcpConnector};

const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_mins(2);
const ACQUIRE_MAX_ATTEMPTS: u32 = 3;

/// Drives the full DORA exchange with exponential-backoff retries until a lease is acquired.
pub struct Manager {
    socket: Socket,
    mac: [u8; 6],
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
        Ok(Self { socket, mac })
    }

    /// Runs DORA with backoff, resolving once a lease is acquired or giving up
    /// with an error after [`ACQUIRE_MAX_ATTEMPTS`] failed attempts.
    ///
    /// # Errors
    ///
    /// Returns the most recent acquisition error once the attempts are exhausted.
    pub async fn acquire(&mut self) -> Result<Lease> {
        acquire_attempts(Run {
            socket: &self.socket,
            mac: self.mac,
        })
        .await
    }

    /// Returns a reference to the underlying raw socket for reuse in rebind operations.
    #[must_use]
    pub fn socket(&self) -> &Socket {
        &self.socket
    }
}

/// Performs the real DORA exchange over the manager's raw socket.
struct Run<'a> {
    socket: &'a Socket,
    mac: [u8; 6],
}

impl DoraAttempt for Run<'_> {
    async fn attempt(&mut self) -> Result<Lease> {
        client::run(self.socket, &self.mac).await
    }
}

/// Produces one DHCP acquisition attempt.
trait DoraAttempt {
    fn attempt(&mut self) -> impl Future<Output = Result<Lease>> + Send;
}

/// Runs the DORA exchange with backoff between attempts, giving up after
/// [`ACQUIRE_MAX_ATTEMPTS`] failures so the supervisor can elect another port.
async fn acquire_attempts<D: DoraAttempt>(mut attempts: D) -> Result<Lease> {
    let mut delay = RETRY_BASE;
    let mut last_error = anyhow::anyhow!("no acquisition was attempted");

    for done in 1..=ACQUIRE_MAX_ATTEMPTS {
        match attempts.attempt().await {
            Ok(lease) => return Ok(lease),
            Err(error) => {
                kmsg::warn!("DHCP failed: {error}; retrying in {}s", delay.as_secs());
                last_error = error;
            }
        }

        if done == ACQUIRE_MAX_ATTEMPTS {
            break;
        }
        sleep(delay).await;
        delay = delay.saturating_mul(2).min(RETRY_MAX);
    }

    Err(last_error)
}

#[cfg(test)]
mod tests {
    use core::net::Ipv4Addr;
    use std::time::{Duration, SystemTime};

    use super::*;

    fn lease() -> Lease {
        Lease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(60),
            renewal_time: Duration::from_secs(30),
            rebind_time: Duration::from_secs(45),
            server_ip: Ipv4Addr::new(192, 168, 1, 1),
            assigned_ip: Ipv4Addr::new(192, 168, 1, 2),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns_servers: Vec::new(),
        }
    }

    struct StubAttempts<'a> {
        counter: &'a mut u32,
        succeed_on: u32,
    }

    impl DoraAttempt for StubAttempts<'_> {
        fn attempt(&mut self) -> impl Future<Output = Result<Lease>> + Send {
            *self.counter = self.counter.saturating_add(1);
            std::future::ready(self.result_for())
        }
    }

    impl StubAttempts<'_> {
        fn result_for(&self) -> Result<Lease> {
            (*self.counter == self.succeed_on)
                .then(lease)
                .ok_or_else(|| anyhow::anyhow!("no server replied"))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_attempts_gives_up_after_max_attempts() {
        // ARRANGE
        let mut attempts = 0_u32;
        let stub = StubAttempts {
            counter: &mut attempts,
            succeed_on: u32::MAX,
        };

        // ACT
        let result = acquire_attempts(stub).await;

        // ASSERT
        result.expect_err("acquisition should have given up");
        assert_eq!(attempts, ACQUIRE_MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_attempts_returns_lease_on_late_success() {
        // ARRANGE
        let mut attempts = 0_u32;
        let stub = StubAttempts {
            counter: &mut attempts,
            succeed_on: 2,
        };

        // ACT
        let result = acquire_attempts(stub).await;

        // ASSERT
        let lease = result.expect("acquisition should have succeeded");
        assert_eq!(attempts, 2);
        assert_eq!(lease.assigned_ip, Ipv4Addr::new(192, 168, 1, 2));
    }
}
