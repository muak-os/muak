//! Lease lifecycle transitions and timer-driven `DHCPv4` exchanges.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use core::time::Duration;

use dhcp_client::Failure as DhcpFailure;
use dhcp_client::lease::Lease;
use tokio::time::{Instant, sleep, sleep_until};

use super::dhcp::Phase;
use super::timer::Deadlines;
use super::{Dhcp, Exchange, intent};
use crate::provision::{Event, Failure, State};

const MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_mins(2);

impl<E: Exchange> Dhcp<E> {
    /// Creates a DHCP method that runs exchanges through `exchange`.
    #[must_use = "the method owns the provisioning state"]
    pub fn new(exchange: E, interface: &str, mac: [u8; 6]) -> Self {
        Self {
            exchange,
            interface: String::from(interface),
            mac,
            phase: Phase::Idle,
            lease: None,
            deadlines: Deadlines::default(),
            queue: VecDeque::new(),
        }
    }

    pub(super) fn start(&mut self) {
        if matches!(self.phase, Phase::Idle | Phase::Stopped | Phase::Degraded) {
            self.begin_acquire();
        }
    }

    pub(super) async fn idle(&mut self) {
        core::future::pending::<()>().await;
    }

    pub(super) fn begin_acquire(&mut self) {
        self.phase = Phase::Acquiring;
        self.deadlines.disarm();
        self.queue.push_back(Event::State(State::Acquiring));
    }

    fn bind(&mut self, lease: Lease) {
        self.deadlines.arm(&lease);
        self.lease = Some(lease);
        self.phase = Phase::Bound;
        intent::configure(self);
        self.queue.push_back(Event::State(State::Bound));
    }

    fn reapply(&mut self) {
        let Some(lease) = self.lease.clone() else {
            return;
        };
        self.deadlines.arm(&lease);
        self.phase = Phase::Bound;
        intent::configure(self);
        self.queue.push_back(Event::State(State::Bound));
    }

    fn expire(&mut self) {
        intent::deconfigure(self);
        self.lease = None;
        self.begin_acquire();
    }

    pub(super) fn stop(&mut self) {
        intent::deconfigure(self);
        self.lease = None;
        self.phase = Phase::Stopped;
        self.deadlines.disarm();
        self.queue.push_back(Event::State(State::Stopped));
    }

    fn degrade(&mut self, error: &DhcpFailure) {
        self.phase = Phase::Degraded;
        self.deadlines.disarm();
        self.queue
            .push_back(Event::Failed(Failure::Protocol(error.to_string())));
    }

    pub(super) fn reconcile(&mut self) {
        if self.lease.is_some() {
            self.reapply();
        } else {
            self.begin_acquire();
        }
    }

    pub(super) fn link_up(&mut self) {
        if !matches!(self.phase, Phase::Degraded) {
            return;
        }
        if self.lease.is_some() {
            self.reapply();
        } else {
            self.begin_acquire();
        }
    }

    pub(super) fn link_down(&mut self) {
        if matches!(self.phase, Phase::Idle | Phase::Stopped) {
            return;
        }
        self.phase = Phase::Degraded;
        self.deadlines.disarm();
        self.queue.push_back(Event::State(State::Degraded));
    }

    pub(super) async fn run_acquire(&mut self) {
        match acquire_with_retry(&self.exchange, &self.interface, self.mac).await {
            Ok(lease) => self.bind(lease),
            Err(error) => self.degrade(&error),
        }
    }

    pub(super) async fn run_renew(&mut self) {
        let Some(lease) = self.lease.clone() else {
            self.begin_acquire();
            return;
        };
        let renewed = self
            .exchange
            .renew(
                &self.interface,
                self.mac,
                lease.server_ip,
                lease.assigned_ip,
            )
            .await;
        match renewed {
            Ok(lease) => self.bind(lease),
            Err(DhcpFailure::Nak) => self.begin_acquire(),
            Err(error) => {
                self.queue.push_back(Event::Warning(format!(
                    "DHCP renew failed on {}: {error}",
                    self.interface
                )));
                self.deadlines.skip_renew();
                self.phase = Phase::Bound;
            }
        }
    }

    pub(super) async fn run_rebind(&mut self) {
        let Some(lease) = self.lease.clone() else {
            self.begin_acquire();
            return;
        };
        let rebound = self
            .exchange
            .rebind(
                &self.interface,
                self.mac,
                lease.server_ip,
                lease.assigned_ip,
            )
            .await;
        match rebound {
            Ok(lease) => self.bind(lease),
            Err(DhcpFailure::Nak) => self.begin_acquire(),
            Err(error) => {
                self.queue.push_back(Event::Warning(format!(
                    "DHCP rebind failed on {}: {error}",
                    self.interface
                )));
                self.deadlines.wait_expiry();
                self.phase = Phase::Bound;
            }
        }
    }

    pub(super) async fn wait_bound(&mut self) {
        let now = Instant::now();
        if self.deadlines.expired(now) {
            self.expire();
        } else if self.deadlines.rebind_due(now) {
            self.phase = Phase::Rebinding;
        } else if self.deadlines.renew_due(now) {
            self.phase = Phase::Renewing;
        } else if let Some(deadline) = self.deadlines.next() {
            sleep_until(deadline).await;
        } else {
            core::future::pending::<()>().await;
        }
    }
}

async fn acquire_with_retry<E: Exchange>(
    exchange: &E,
    interface: &str,
    mac: [u8; 6],
) -> dhcp_client::Result<Lease> {
    let mut delay = RETRY_BASE;
    let mut failure = DhcpFailure::Timeout;

    for attempt in 1..=MAX_ATTEMPTS {
        match exchange.acquire(interface, mac).await {
            Ok(lease) => return Ok(lease),
            Err(error) if attempt == MAX_ATTEMPTS => return Err(error),
            Err(error) => failure = error,
        }
        sleep(delay).await;
        delay = delay.saturating_mul(2).min(RETRY_MAX);
    }

    Err(failure)
}
