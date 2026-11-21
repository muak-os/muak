//! Deadline computation and expiry handling for the SLAAC method.

use core::net::IpAddr;
use core::time::Duration;

use tokio::time::Instant;

use super::state::{AddressState, Expiring};
use super::{Slaac, deconfigure};
use crate::provision::{Event, Intent};
use crate::transport::icmpv6::Transport;

const IDLE_HORIZON: Duration = Duration::from_hours(1);

pub(super) fn next_deadline<T: Transport>(method: &Slaac<T>) -> Instant {
    let mut deadline = Instant::now()
        .checked_add(IDLE_HORIZON)
        .unwrap_or_else(Instant::now);

    if let Some(address) = method.address.as_ref() {
        if address.state == AddressState::Preferred && address.preferred_until < deadline {
            deadline = address.preferred_until;
        }
        if address.valid_until < deadline {
            deadline = address.valid_until;
        }
    }

    if let Some(router) = method.router.as_ref()
        && router.expires_at < deadline
    {
        deadline = router.expires_at;
    }

    for dns in &method.dns_servers {
        if dns.expires_at < deadline {
            deadline = dns.expires_at;
        }
    }

    deadline
}

pub(super) fn handle_expiration<T: Transport>(method: &mut Slaac<T>) {
    let now = Instant::now();

    if let Some(address) = method.address.as_mut()
        && address.state == AddressState::Preferred
        && !address.is_preferred()
    {
        address.state = AddressState::Deprecated;
        return;
    }

    if let Some(address) = method.address.as_ref()
        && !address.is_valid()
    {
        let intent = deconfigure(address.address, address.prefix_len);
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
    method.dns_servers.retain(Expiring::is_valid);
    if had_dns && method.dns_servers.is_empty() {
        method.dns_servers = method.dns_fallback();
    }
}
