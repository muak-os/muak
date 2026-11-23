//! Router advertisement processing after initial acquisition.

use core::net::IpAddr;
use std::time::Instant;

use addrconf::{Event, Intent};

use super::Slaac;
use super::solicit::process_initial_ra;
use super::state::ManagedDns;
use super::transport::Transport;
use crate::address::generate;
use crate::icmpv6::types::RouterAdvertisement;

pub(super) fn process_unsolicited_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
    now: Instant,
) {
    if ra.router_lifetime == 0 {
        if let Some(router) = method.router.as_ref()
            && router.value == ra.source
        {
            method.router = None;
            method
                .queue
                .push_back(Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(
                    ra.source,
                ))));
        }
        return;
    }

    if method.address.is_none() || method.router.is_none() {
        process_initial_ra(method, ra, now);
        return;
    }

    if let Some(router) = method.router.as_mut()
        && router.value == ra.source
    {
        router.refresh_lifetime(u64::from(ra.router_lifetime), now);
    }

    refresh_address_from_ra(method, ra, now);

    if !ra.dns_servers.is_empty() {
        refresh_dns_from_ra(method, ra, now);
    }
}

fn refresh_address_from_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
    now: Instant,
) {
    let Some(address) = method.address.as_mut() else {
        return;
    };

    for prefix in &ra.prefixes {
        let generated = generate(prefix.prefix, prefix.prefix_len, &method.mac);
        if prefix.autonomous
            && prefix.prefix_len <= 64
            && address.router == ra.source
            && generated == Some(address.address)
        {
            address.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime, now);
            return;
        }
    }
}

fn refresh_dns_from_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
    now: Instant,
) {
    for server in &ra.dns_servers {
        if let Some(managed) = method
            .dns_servers
            .iter_mut()
            .find(|dns| dns.value == *server)
        {
            managed.refresh_lifetime(u64::from(ra.dns_lifetime), now);
        } else {
            method
                .dns_servers
                .push(ManagedDns::new(*server, u64::from(ra.dns_lifetime), now));
        }
    }
}
