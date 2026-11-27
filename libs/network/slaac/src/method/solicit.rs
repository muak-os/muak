//! Router solicitation lifecycle and stop teardown.

use core::net::IpAddr;
use std::time::Instant;

use ip_method::{Event, Intent};
use netmodel::MethodState;

use super::{Action, intent};
use crate::{MAX_RTR_SOLICITATIONS, RTR_SOLICITATION_INTERVAL, Slaac};

pub(crate) fn start(method: &mut Slaac, now: Instant) {
    method.running = true;
    method.solicited = false;
    method.solicitation_attempt = 0;
    method.solicitation_at = None;
    method.retry_at = None;
    if method.open {
        method
            .queue
            .push_back(Action::Emit(Event::State(MethodState::Acquiring)));
        schedule(method, now);
    } else {
        method.queue.push_back(Action::Open);
    }
}

pub(crate) fn suspend(method: &mut Slaac) {
    if !method.running {
        return;
    }
    method.running = false;
    method
        .queue
        .push_back(Action::Emit(Event::State(MethodState::Degraded)));
}

pub(crate) fn stop(method: &mut Slaac) {
    method.running = false;
    method.solicited = true;
    method.solicitation_at = None;
    method.retry_at = None;
    if let Some(address) = method.address.take() {
        let intent = intent::deconfigure(address.address, address.prefix_len);
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }
    if let Some(router) = method.router.take() {
        let intent = Intent::RemoveDefaultRoute(IpAddr::V6(router.value));
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }
    method.dns_servers.clear();
    method
        .queue
        .push_back(Action::Emit(Event::State(MethodState::Stopped)));
    if method.open {
        method.queue.push_back(Action::Close);
        method.open = false;
    }
}

pub(crate) fn schedule(method: &mut Slaac, now: Instant) {
    method.solicitation_attempt = method.solicitation_attempt.saturating_add(1);
    method.solicitation_at = now.checked_add(RTR_SOLICITATION_INTERVAL);
    method.queue.push_back(Action::SendSolicitation);
}

pub(crate) fn exhaust(method: &mut Slaac) {
    method.solicited = true;
    method.solicitation_at = None;
    method.warn(format!(
        "no Router Advertisement on {} after {MAX_RTR_SOLICITATIONS} attempts, continuing to listen",
        method.link.name
    ));
    method
        .queue
        .push_back(Action::Emit(Event::State(MethodState::Degraded)));
}
