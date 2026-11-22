//! Lease lifecycle transitions of the `DHCPv4` method.

use alloc::collections::VecDeque;
use alloc::string::String;
use std::time::Instant;

use dhcp_client::Failure as DhcpFailure;
use dhcp_client::lease::Lease;

use super::dhcp::Phase;
use super::timer::Deadlines;
use super::{Dhcp, Exchange, intent};
use crate::provision::{Event, Failure, State, Step};

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
            retry_at: None,
            attempts: 0,
            queue: VecDeque::new(),
        }
    }

    pub(super) fn start(&mut self) {
        if matches!(self.phase, Phase::Idle | Phase::Stopped | Phase::Degraded) {
            self.begin_acquire();
        }
    }

    pub(super) fn begin_acquire(&mut self) {
        self.phase = Phase::Acquiring;
        self.deadlines.disarm();
        self.retry_at = None;
        self.attempts = 0;
        self.queue.push_back(Event::State(State::Acquiring));
    }

    pub(super) fn stop(&mut self) {
        intent::deconfigure(self);
        self.lease = None;
        self.phase = Phase::Stopped;
        self.deadlines.disarm();
        self.retry_at = None;
        self.attempts = 0;
        self.queue.push_back(Event::State(State::Stopped));
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
        self.retry_at = None;
        self.attempts = 0;
        self.queue.push_back(Event::State(State::Degraded));
    }

    pub(super) fn wait_bound(&mut self, now: Instant) -> Step {
        if self.deadlines.next().is_none()
            && let Some(lease) = self.lease.as_ref()
        {
            self.deadlines.arm(now, lease);
        }
        if self.deadlines.expired(now) {
            self.expire();
            return Step::Advance;
        }
        if self.deadlines.rebind_due(now) {
            self.deadlines.wait_expiry();
            self.phase = Phase::Rebinding;
            return Step::Advance;
        }
        if self.deadlines.renew_due(now) {
            self.deadlines.skip_renew();
            self.phase = Phase::Renewing;
            return Step::Advance;
        }

        Step::Wait
    }

    pub(super) fn bind(&mut self, lease: Lease) {
        // The deadlines arm on the next bound step so they anchor to the clock
        // read after the exchange instead of the one read before it. Disarming
        // first drops the deadlines of the cycle that produced this bind, so a
        // renewed or rebound lease is re-anchored instead of keeping the stale
        // acquisition deadlines.
        self.deadlines.disarm();
        self.lease = Some(lease);
        self.phase = Phase::Bound;
        self.retry_at = None;
        self.attempts = 0;
        intent::configure(self);
        self.queue.push_back(Event::State(State::Bound));
    }

    pub(super) fn degrade(&mut self, error: &DhcpFailure) {
        self.phase = Phase::Degraded;
        self.deadlines.disarm();
        self.queue
            .push_back(Event::Failed(Failure::Protocol(error.to_string())));
    }

    fn reapply(&mut self) {
        if self.lease.is_none() {
            return;
        }
        self.deadlines.disarm();
        self.phase = Phase::Bound;
        intent::configure(self);
        self.queue.push_back(Event::State(State::Bound));
    }

    pub(super) fn expire(&mut self) {
        intent::deconfigure(self);
        self.lease = None;
        self.begin_acquire();
    }
}
