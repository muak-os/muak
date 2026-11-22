//! Deadline computation and expiry handling for the SLAAC method.

use core::net::IpAddr;
use std::time::Instant;

use super::state::AddressState;
use super::{RTR_SOLICITATION_INTERVAL, Slaac, intent};
use crate::provision::{Event, Intent};
use crate::transport::icmpv6::Transport;

pub(super) fn next_wake<T: Transport>(method: &Slaac<T>, now: Instant) -> Option<Instant> {
    if !method.running {
        return None;
    }
    if !method.solicited {
        // While the next solicitation is not yet armed, the step about to run
        // sends one and arms it, so that interval is the earliest wake.
        return match method.solicitation_at {
            Some(at) if at > now => Some(at),
            _ => now.checked_add(RTR_SOLICITATION_INTERVAL),
        };
    }

    next_deadline(method)
}

pub(super) fn next_deadline<T: Transport>(method: &Slaac<T>) -> Option<Instant> {
    let address = method.address.as_ref();
    let preferred = address
        .filter(|address| address.state == AddressState::Preferred)
        .map(|address| address.preferred_until);
    let valid = address.map(|address| address.valid_until);
    let router = method.router.as_ref().map(|router| router.expires_at);
    let dns = method.dns_servers.iter().map(|dns| dns.expires_at).min();

    [preferred, valid, router, dns].into_iter().flatten().min()
}

pub(super) fn due<T: Transport>(method: &Slaac<T>, now: Instant) -> bool {
    next_deadline(method).is_some_and(|at| at <= now)
}

pub(super) fn handle_expiration<T: Transport>(method: &mut Slaac<T>, now: Instant) {
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
        method.queue.push_back(Event::Intent(intent));
        return;
    }

    if let Some(router) = method.router.as_ref()
        && now >= router.expires_at
    {
        let router = router.value;
        method.router = None;
        method
            .queue
            .push_back(Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(
                router,
            ))));
        return;
    }

    let had_dns = !method.dns_servers.is_empty();
    method.dns_servers.retain(|dns| dns.is_valid(now));
    if had_dns && method.dns_servers.is_empty() {
        method.dns_servers = method.dns_fallback(now);
    }
}
