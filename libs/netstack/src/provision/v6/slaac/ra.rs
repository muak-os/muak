//! Router advertisement processing after initial acquisition.

use core::net::IpAddr;

use ::slaac::address::generate;
use ::slaac::icmpv6::types::RouterAdvertisement;

use super::Slaac;
use super::solicit::process_initial_ra;
use super::state::ManagedDns;
use crate::provision::{Event, Intent};
use crate::transport::icmpv6::Transport;

pub(super) fn process_unsolicited_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
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
        process_initial_ra(method, ra);
        return;
    }

    if let Some(router) = method.router.as_mut()
        && router.value == ra.source
    {
        router.refresh_lifetime(u64::from(ra.router_lifetime));
    }

    refresh_address_from_ra(method, ra);

    if !ra.dns_servers.is_empty() {
        refresh_dns_from_ra(method, ra);
    }
}

fn refresh_address_from_ra<T: Transport>(method: &mut Slaac<T>, ra: &RouterAdvertisement) {
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
            address.refresh_lifetimes(prefix.valid_lifetime, prefix.preferred_lifetime);
            return;
        }
    }
}

fn refresh_dns_from_ra<T: Transport>(method: &mut Slaac<T>, ra: &RouterAdvertisement) {
    for server in &ra.dns_servers {
        if let Some(managed) = method
            .dns_servers
            .iter_mut()
            .find(|dns| dns.value == *server)
        {
            managed.refresh_lifetime(u64::from(ra.dns_lifetime));
        } else {
            method
                .dns_servers
                .push(ManagedDns::new(*server, u64::from(ra.dns_lifetime)));
        }
    }
}
