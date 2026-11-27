//! Start, stop and link transitions of the `DHCPv4` method.

use ip_method::Event;
use netmodel::MethodState;

use crate::effects::Action;
use crate::exchange::Exchange;
use crate::{Dhcp, Phase, acquire, exchange, intent};

pub(super) fn start(method: &mut Dhcp) {
    if matches!(method.phase, Phase::Idle | Phase::Stopped | Phase::Degraded) {
        acquire::begin(method);
    }
}

pub(super) fn stop(method: &mut Dhcp) {
    intent::deconfigure(method);
    method.lease = None;
    method.bound_at = None;
    method.phase = Phase::Stopped;
    method.deadlines.disarm();
    method.retry_at = None;
    method.attempts = 0;
    method.exchange = Exchange::Idle;
    exchange::release_all(method);
    let state = Event::State(MethodState::Stopped);
    method.queue.push_back(Action::Event(state));
}

pub(super) fn reconcile(method: &mut Dhcp) {
    if matches!(method.phase, Phase::Stopped) {
        return;
    }
    if method.lease.is_some() {
        reapply(method);
    } else {
        acquire::begin(method);
    }
}

pub(super) fn link_up(method: &mut Dhcp) {
    if !matches!(method.phase, Phase::Degraded) {
        return;
    }
    if method.lease.is_some() {
        reapply(method);
    } else {
        acquire::begin(method);
    }
}

pub(super) fn link_down(method: &mut Dhcp) {
    if matches!(method.phase, Phase::Idle | Phase::Stopped) {
        return;
    }
    method.phase = Phase::Degraded;
    method.deadlines.disarm();
    method.retry_at = None;
    method.attempts = 0;
    method.exchange = Exchange::Idle;
    exchange::release_all(method);
    let state = Event::State(MethodState::Degraded);
    method.queue.push_back(Action::Event(state));
}

fn reapply(method: &mut Dhcp) {
    if method.lease.is_none() {
        return;
    }
    method.deadlines.disarm();
    method.phase = Phase::Bound;
    method.exchange = Exchange::Idle;
    intent::configure(method);
    let state = Event::State(MethodState::Bound);
    method.queue.push_back(Action::Event(state));
}
