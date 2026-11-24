//! Acquisition, renewal and rebinding paths of the `DHCPv4` method.

use alloc::format;
use core::net::Ipv4Addr;
use std::time::Instant;

use ip_method::{Event, SocketSlot};

use crate::effects::Action;
use crate::exchange::{Exchange, Transmit};
use crate::fault::Fault;
use crate::{Dhcp, Phase, exchange, intent, retry};

// Starts discovery and clears exchange state.
pub(super) fn begin(method: &mut Dhcp) {
    method.phase = Phase::Acquiring;
    method.deadlines.disarm();
    method.retry_at = None;
    method.attempts = 0;
    method.exchange = Exchange::Idle;
    let state = Event::State(ip_method::State::Acquiring);
    method.queue.push_back(Action::Event(state));
    exchange::begin(method, Transmit::Discover);
}

// Drops the lease and starts discovery again.
pub(super) fn expire(method: &mut Dhcp) {
    intent::deconfigure(method);
    method.lease = None;
    method.bound_at = None;
    begin(method);
}

// Starts renewing with the granting server.
pub(super) fn start_renew(method: &mut Dhcp) {
    let Some((server, assigned)) = endpoints(method) else {
        begin(method);
        return;
    };
    method.phase = Phase::Renewing;
    method.deadlines.skip_renew();
    exchange::begin(method, Transmit::Renew { server, assigned });
}

// Starts rebinding by broadcast.
pub(super) fn start_rebind(method: &mut Dhcp) {
    let Some((server, assigned)) = endpoints(method) else {
        begin(method);
        return;
    };
    method.phase = Phase::Rebinding;
    method.deadlines.wait_expiry();
    exchange::begin(method, Transmit::Rebind { server, assigned });
}

// Retries or degrades after a failed acquisition attempt.
pub(super) fn failed(method: &mut Dhcp, now: Instant, fault: Fault) {
    method.attempts = method.attempts.saturating_add(1);
    method.exchange = Exchange::Idle;
    match retry::classify(method.attempts) {
        retry::Outcome::Retry(delay) => {
            method.retry_at = now.checked_add(delay);
            let warning = format!(
                "DHCP acquire failed on {} after attempt {}, retrying",
                method.link.name, method.attempts
            );
            method
                .queue
                .push_back(Action::Event(Event::Warning(warning)));
        }
        retry::Outcome::Failed => {
            method.attempts = 0;
            crate::state::degrade(method, fault);
        }
    }
}

// Drops the refused lease and falls back to rebinding after a failed renewal.
pub(super) fn renew_failed(method: &mut Dhcp, fault: Fault) {
    method.exchange = Exchange::Idle;
    exchange::release(method, SocketSlot::Secondary);
    if fault.is_nak() {
        expire(method);
        return;
    }
    let warning = format!("DHCP renew failed on {}: {fault}", method.link.name);
    method
        .queue
        .push_back(Action::Event(Event::Warning(warning)));
    method.deadlines.skip_renew();
    method.phase = Phase::Bound;
}

// Drops the refused lease and waits for expiry after a failed rebinding.
pub(super) fn rebind_failed(method: &mut Dhcp, fault: Fault) {
    method.exchange = Exchange::Idle;
    exchange::release(method, SocketSlot::Primary);
    if fault.is_nak() {
        expire(method);
        return;
    }
    let warning = format!("DHCP rebind failed on {}: {fault}", method.link.name);
    method
        .queue
        .push_back(Action::Event(Event::Warning(warning)));
    method.deadlines.wait_expiry();
    method.phase = Phase::Bound;
}

// Returns the server and assigned address of the held lease.
fn endpoints(method: &Dhcp) -> Option<(Ipv4Addr, Ipv4Addr)> {
    method
        .lease
        .as_ref()
        .map(|lease| (lease.server_ip, lease.assigned_ip))
}
