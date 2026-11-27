//! Router advertisement reception, initial acquisition, and refresh.

use core::net::{IpAddr, Ipv6Addr};
use std::time::Instant;

use ip_method::{Event, Intent};
use ndp::message::RouterAdvertisement;
use ndp::types::Option;
use netmodel::MethodState;

use super::state::{ManagedAddress, ManagedDns, ManagedRouter};
use super::{Action, intent};
use crate::{Slaac, address};

pub(crate) fn receive(method: &mut Slaac, source: Ipv6Addr, message: &[u8], now: Instant) {
    if message.first().copied() != Some(ndp::ROUTER_ADVERTISEMENT) {
        return;
    }
    let Ok(ra) = ndp::parse::router_advertisement(message) else {
        method.warn(String::from("malformed Router Advertisement"));
        return;
    };
    if method.solicited {
        update(method, source, &ra, now);
        return;
    }

    method.solicited = true;
    method.solicitation_at = None;
    if !initial(method, source, &ra, now) {
        method.warn(format!(
            "no usable autonomous prefix on {}, continuing to listen",
            method.link.name
        ));
    }
}

fn initial(
    method: &mut Slaac,
    source: Ipv6Addr,
    ra: &RouterAdvertisement<'_>,
    now: Instant,
) -> bool {
    method.dns_servers = intent::resolve_dns(method, ra, now);

    let Some(prefix) = ra.options.clone().find_map(|option| match option {
        Option::PrefixInformation(prefix) if prefix.autonomous && prefix.prefix_len <= 64 => {
            Some(prefix)
        }
        Option::SourceLinkLayer(_)
        | Option::TargetLinkLayer(_)
        | Option::PrefixInformation(_)
        | Option::Mtu(_)
        | Option::RecursiveDnsServers(_) => None,
    }) else {
        return false;
    };
    let Some(address) = address::generate(
        prefix.prefix,
        prefix.prefix_len,
        method.link.mac.into_array(),
    ) else {
        return false;
    };
    let router = (ra.header.router_lifetime != 0)
        .then(|| ManagedRouter::new(source, u64::from(ra.header.router_lifetime), now));

    if let Some(old) = method.address.take() {
        let intent = intent::deconfigure(old.address, old.prefix_len);
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }
    if let Some(old) = method.router.take()
        && router.as_ref().is_none_or(|new| new.value != old.value)
    {
        let intent = Intent::RemoveDefaultRoute(IpAddr::V6(old.value));
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }

    method.address = Some(ManagedAddress::new(
        address,
        prefix.prefix_len,
        source,
        prefix.valid_lifetime,
        prefix.preferred_lifetime,
        now,
    ));
    method.router = router;

    let intent = intent::configure(address, prefix.prefix_len);
    method.queue.push_back(Action::Emit(Event::Intent(intent)));
    if ra.header.router_lifetime != 0 {
        let intent = Intent::EnsureDefaultRoute(IpAddr::V6(source));
        method.queue.push_back(Action::Emit(Event::Intent(intent)));
    }
    method
        .queue
        .push_back(Action::Emit(Event::State(MethodState::Bound)));

    true
}

fn update(method: &mut Slaac, source: Ipv6Addr, ra: &RouterAdvertisement<'_>, now: Instant) {
    if ra.header.router_lifetime == 0 {
        if method
            .router
            .as_ref()
            .is_some_and(|router| router.value == source)
        {
            method.router = None;
            let intent = Intent::RemoveDefaultRoute(IpAddr::V6(source));
            method.queue.push_back(Action::Emit(Event::Intent(intent)));
        }
        refresh_dns(method, ra, now);
        return;
    }
    if method.address.is_none() || method.router.is_none() {
        initial(method, source, ra, now);
        return;
    }
    if let Some(router) = method.router.as_mut()
        && router.value == source
    {
        router.refresh_lifetime(u64::from(ra.header.router_lifetime), now);
    }
    refresh_address(method, source, ra, now);
    refresh_dns(method, ra, now);
}

fn refresh_address(
    method: &mut Slaac,
    source: Ipv6Addr,
    ra: &RouterAdvertisement<'_>,
    now: Instant,
) {
    let mac = method.link.mac.into_array();
    let Some(address) = method.address.as_mut() else {
        return;
    };
    for option in ra.options.clone() {
        let Option::PrefixInformation(prefix) = option else {
            continue;
        };
        let generated = address::generate(prefix.prefix, prefix.prefix_len, mac);
        if prefix.autonomous
            && prefix.prefix_len <= 64
            && address.router == source
            && generated == Some(address.address)
        {
            address.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime, now);
            return;
        }
    }
}

fn refresh_dns(method: &mut Slaac, ra: &RouterAdvertisement<'_>, now: Instant) {
    for option in ra.options.clone() {
        let Option::RecursiveDnsServers(dns) = option else {
            continue;
        };
        let lifetime = u64::from(dns.lifetime);
        for address in dns.addresses {
            refresh_server(method, address, lifetime, now);
        }
    }
    if method.dns_servers.is_empty() {
        method.dns_servers = intent::fallback(method, now);
    }
}

fn refresh_server(method: &mut Slaac, address: Ipv6Addr, lifetime: u64, now: Instant) {
    if lifetime == 0 {
        method
            .dns_servers
            .retain(|managed| managed.value != address);
        return;
    }
    if let Some(managed) = method
        .dns_servers
        .iter_mut()
        .find(|managed| managed.value == address)
    {
        managed.refresh_lifetime(lifetime, now);
        return;
    }
    method
        .dns_servers
        .push(ManagedDns::new(address, lifetime, now));
}
