//! Provisioning intents and DNS state derived from router advertisements.

use core::net::{IpAddr, Ipv6Addr};
use std::time::Instant;

use ip_method::{AddressConfig, Event, Intent};
use ndp::message::RouterAdvertisement;
use ndp::types::Option;

use super::Action;
use super::state::ManagedDns;
use crate::Slaac;

pub(crate) const fn configure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Configure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

pub(crate) const fn deconfigure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Deconfigure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

pub(crate) fn reapply(method: &mut Slaac, now: Instant) {
    if !method.running {
        return;
    }
    let Some(address) = method
        .address
        .as_ref()
        .filter(|address| address.is_valid(now))
    else {
        return;
    };
    let intent = configure(address.address, address.prefix_len);
    method.queue.push_back(Action::Emit(Event::Intent(intent)));
    if let Some(router) = method.router.as_ref().filter(|router| router.is_valid(now)) {
        let intent = Intent::EnsureDefaultRoute(IpAddr::V6(router.value));
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }
}

pub(crate) fn resolve_dns(
    method: &Slaac,
    ra: &RouterAdvertisement<'_>,
    now: Instant,
) -> Vec<ManagedDns> {
    let mut servers = Vec::new();
    for option in ra.options.clone() {
        let Option::RecursiveDnsServers(dns) = option else {
            continue;
        };
        let lifetime = u64::from(dns.lifetime);
        if lifetime == 0 {
            continue;
        }
        for address in dns.addresses {
            servers.push(ManagedDns::new(address, lifetime, now));
        }
    }
    if servers.is_empty() {
        return fallback(method, now);
    }

    servers
}

pub(crate) fn fallback(method: &Slaac, now: Instant) -> Vec<ManagedDns> {
    method
        .fallback_dns
        .iter()
        .map(|&server| ManagedDns::new(server, u64::from(u32::MAX), now))
        .collect()
}
