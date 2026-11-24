//! Lease lifecycle transitions of the `DHCPv4` method.

use alloc::collections::VecDeque;
use std::time::Instant;

use ip_method::{Command, Event, Failure, State};

use crate::effects::Action;
use crate::exchange::{Exchange, SocketState, Transmit};
use crate::fault::Fault;
use crate::timer::Deadlines;
use crate::{Dhcp, Lease, MAX_FRAME_LEN, MAX_MESSAGE_LEN, Phase, acquire, lifecycle};

impl Dhcp {
    /// Creates a DHCP method that provisions `link` from `config`.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(link: ip_method::LinkInfo, config: netmodel::Ipv4Config) -> Self {
        Self {
            link,
            config,
            phase: Phase::Idle,
            lease: None,
            bound_at: None,
            deadlines: Deadlines::new(),
            retry_at: None,
            attempts: 0,
            exchange: Exchange::Idle,
            primary: SocketState::Closed,
            secondary: SocketState::Closed,
            xid: 0,
            queue: VecDeque::new(),
            message: [0; MAX_MESSAGE_LEN],
            frame: [0; MAX_FRAME_LEN],
        }
    }

    /// Returns the link the method runs on.
    #[must_use = "callers need the link to address method effects"]
    pub const fn link(&self) -> &ip_method::LinkInfo {
        &self.link
    }

    /// Returns the configuration the method was created with.
    #[must_use = "callers need the configuration to report it"]
    pub const fn config(&self) -> &netmodel::Ipv4Config {
        &self.config
    }
}

pub(super) fn command(method: &mut Dhcp, command: Command) {
    match command {
        Command::Start => lifecycle::start(method),
        Command::Stop => lifecycle::stop(method),
        Command::Reconcile => lifecycle::reconcile(method),
        Command::LinkUp => lifecycle::link_up(method),
        Command::LinkDown => lifecycle::link_down(method),
    }
}

pub(super) fn on_link(method: &mut Dhcp, link: ip_method::LinkInfo) {
    let was_up = method.link.state.has_carrier();
    method.link = link;
    let is_up = method.link.state.has_carrier();

    match (was_up, is_up) {
        (true, false) => lifecycle::link_down(method),
        (false, true) => lifecycle::link_up(method),
        (true, true) | (false, false) => {}
    }
}

pub(super) fn advance(method: &mut Dhcp, now: Instant) {
    while retry(method, now) || reply(method, now) || lease(method, now) {}
}

pub(super) fn failure(method: &mut Dhcp, now: Instant, kind: Transmit, fault: Fault) {
    match kind {
        Transmit::Discover | Transmit::Select { .. } => acquire::failed(method, now, fault),
        Transmit::Renew { .. } => acquire::renew_failed(method, fault),
        Transmit::Rebind { .. } => acquire::rebind_failed(method, fault),
    }
}

pub(super) fn bind(method: &mut Dhcp, now: Instant, kind: Transmit, lease: Lease) {
    method.bound_at = Some(now);
    method.deadlines.arm(now, &lease);
    method.exchange = Exchange::Idle;
    method.retry_at = None;
    method.attempts = 0;
    crate::exchange::release(method, kind.slot());
    if let Some(previous) = method.lease.as_ref()
        && replaced(previous, &lease)
    {
        crate::intent::deconfigure(method);
    }
    method.lease = Some(lease);
    method.phase = Phase::Bound;
    crate::intent::configure(method);
    let state = Event::State(State::Bound);
    method.queue.push_back(Action::Event(state));
}

fn replaced(previous: &Lease, next: &Lease) -> bool {
    previous.assigned_ip != next.assigned_ip
        || previous.prefix_len != next.prefix_len
        || previous.gateway != next.gateway
}

pub(super) fn degrade(method: &mut Dhcp, fault: Fault) {
    method.phase = Phase::Degraded;
    method.deadlines.disarm();
    method.retry_at = None;
    method.exchange = Exchange::Idle;
    crate::exchange::release_all(method);
    let failure = Failure::Protocol(fault.to_string());
    method
        .queue
        .push_back(Action::Event(Event::Failed(failure)));
}

fn retry(method: &mut Dhcp, now: Instant) -> bool {
    let Some(at) = method.retry_at else {
        return false;
    };
    if now < at {
        return false;
    }
    method.retry_at = None;
    crate::exchange::begin(method, Transmit::Discover);

    true
}

fn reply(method: &mut Dhcp, now: Instant) -> bool {
    let Exchange::Reply { kind, deadline } = method.exchange else {
        return false;
    };
    if now < deadline {
        return false;
    }
    method.exchange = Exchange::Idle;
    failure(method, now, kind, Fault::Timeout);

    true
}

fn lease(method: &mut Dhcp, now: Instant) -> bool {
    if method.phase == Phase::Degraded {
        return expired_while_down(method, now);
    }
    if method.phase != Phase::Bound {
        return false;
    }
    arm(method, now);
    if method.deadlines.expired(now) {
        acquire::expire(method);
        return true;
    }
    if method.deadlines.rebind_due(now) {
        acquire::start_rebind(method);
        return true;
    }
    if method.deadlines.renew_due(now) {
        acquire::start_renew(method);
        return true;
    }

    false
}

fn expired_while_down(method: &mut Dhcp, now: Instant) -> bool {
    arm(method, now);
    if !method.deadlines.expired(now) {
        return false;
    }
    crate::intent::deconfigure(method);
    method.lease = None;
    method.bound_at = None;
    method.deadlines.disarm();

    true
}

fn arm(method: &mut Dhcp, now: Instant) {
    if method.deadlines.next().is_none()
        && let Some(lease) = method.lease.as_ref()
    {
        let anchor = method.bound_at.unwrap_or(now);
        method.deadlines.arm(anchor, lease);
    }
}
