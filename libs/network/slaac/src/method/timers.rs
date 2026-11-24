//! Deadline computation and lifetime expiry handling.

use core::net::IpAddr;
use std::time::Instant;

use ip_method::{Event, Intent};

use super::state::AddressState;
use super::{Action, intent};
use crate::{RTR_SOLICITATION_INTERVAL, Slaac};

pub(crate) fn next_wake(method: &Slaac, now: Instant) -> Option<Instant> {
    if !method.running {
        return None;
    }
    let retry = method.retry_at.map(|at| at.max(now));
    if !method.open {
        return retry;
    }

    let solicitation = if method.solicited {
        next_deadline(method)
    } else {
        match method.solicitation_at {
            Some(at) if at > now => Some(at),
            _ => now.checked_add(RTR_SOLICITATION_INTERVAL),
        }
    };

    [solicitation, retry].into_iter().flatten().min()
}

pub(crate) fn due(method: &Slaac, now: Instant) -> bool {
    next_deadline(method).is_some_and(|at| at <= now)
}

pub(crate) fn handle_expiration(method: &mut Slaac, now: Instant) {
    if let Some(address) = method.address.as_mut()
        && address.state == AddressState::Preferred
        && !address.is_preferred(now)
    {
        address.state = AddressState::Deprecated;
        return;
    }

    if let Some(address) = method.address.as_ref()
        && !address.is_valid(now)
    {
        let intent = intent::deconfigure(address.address, address.prefix_len);
        method.address = None;
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
        return;
    }

    if let Some(router) = method.router.as_ref()
        && now >= router.expires_at
    {
        let router = router.value;
        method.router = None;
        let intent = Intent::RemoveDefaultRoute(IpAddr::V6(router));
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
        return;
    }

    let had_dns = !method.dns_servers.is_empty();
    method.dns_servers.retain(|dns| dns.is_valid(now));
    if had_dns && method.dns_servers.is_empty() {
        method.dns_servers = intent::fallback(method, now);
    }
}

fn next_deadline(method: &Slaac) -> Option<Instant> {
    let address = method.address.as_ref();
    let preferred = address
        .filter(|address| address.state == AddressState::Preferred)
        .map(|address| address.preferred_until);
    let valid = address.map(|address| address.valid_until);
    let router = method.router.as_ref().map(|router| router.expires_at);
    let dns = method.dns_servers.iter().map(|dns| dns.expires_at).min();

    [preferred, valid, router, dns].into_iter().flatten().min()
}
