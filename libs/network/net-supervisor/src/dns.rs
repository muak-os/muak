//! Resolver aggregation from the layer 3 owner, with config fallback.

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use core::net::IpAddr;
use core::net::{Ipv4Addr, Ipv6Addr};

use crate::actions::Action;
use crate::{Supervisor, owner};

/// Emits a resolver write when the desired nameservers changed.
pub(crate) fn flush(supervisor: &mut Supervisor) {
    let Some(owner) = owner::owner(supervisor).cloned() else {
        return;
    };
    let v4 = desired_v4(supervisor, &owner);
    let v6 = desired_v6(supervisor, &owner);
    if v4 == supervisor.resolver_v4 && v6 == supervisor.resolver_v6 {
        return;
    }

    supervisor.resolver_v4 = v4;
    supervisor.resolver_v6 = v6;
    supervisor.queue.push_back(Action::WriteResolver);
}

#[cfg(feature = "ipv4")]
fn desired_v4(supervisor: &Supervisor, owner: &netmodel::Name) -> Vec<Ipv4Addr> {
    let mut dns = match supervisor
        .snapshots
        .get(owner)
        .and_then(|snapshot| snapshot.v4.as_ref())
    {
        Some(view) => v4_view_dns(view),
        None => Vec::new(),
    };
    if dns.is_empty() {
        dns = config_v4(supervisor);
    }
    dedup(&mut dns);

    dns
}

#[cfg(not(feature = "ipv4"))]
fn desired_v4(_supervisor: &Supervisor, _owner: &netmodel::Name) -> Vec<Ipv4Addr> {
    Vec::new()
}

#[cfg(feature = "ipv6")]
fn desired_v6(supervisor: &Supervisor, owner: &netmodel::Name) -> Vec<Ipv6Addr> {
    let mut dns = match supervisor
        .snapshots
        .get(owner)
        .and_then(|snapshot| snapshot.v6.as_ref())
    {
        Some(view) => v6_view_dns(view),
        None => Vec::new(),
    };
    if dns.is_empty() {
        dns = config_v6(supervisor);
    }
    dedup(&mut dns);

    dns
}

#[cfg(not(feature = "ipv6"))]
fn desired_v6(_supervisor: &Supervisor, _owner: &netmodel::Name) -> Vec<Ipv6Addr> {
    Vec::new()
}

#[cfg(feature = "ipv4")]
fn config_v4(supervisor: &Supervisor) -> Vec<Ipv4Addr> {
    supervisor
        .config
        .dns
        .iter()
        .filter_map(|address| match *address {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        })
        .collect()
}

#[cfg(feature = "ipv6")]
fn config_v6(supervisor: &Supervisor) -> Vec<Ipv6Addr> {
    supervisor
        .config
        .dns
        .iter()
        .filter_map(|address| match *address {
            IpAddr::V6(address) => Some(address),
            IpAddr::V4(_) => None,
        })
        .collect()
}

#[cfg(feature = "ipv4")]
fn v4_view_dns(view: &iface::V4View) -> Vec<Ipv4Addr> {
    match *view {
        iface::V4View::Static(ref view) => view.dns.clone(),
        iface::V4View::Dhcp(ref view) => view.lease.dns_servers.clone(),
    }
}

#[cfg(feature = "ipv6")]
fn v6_view_dns(view: &iface::V6View) -> Vec<Ipv6Addr> {
    match *view {
        iface::V6View::Static(ref view) => view.dns.clone(),
        iface::V6View::Slaac(ref view) => view.dns.clone(),
    }
}

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
fn dedup<T: PartialEq>(values: &mut Vec<T>) {
    let mut unique: Vec<T> = Vec::with_capacity(values.len());
    values.drain(..).for_each(|value| {
        if !unique.contains(&value) {
            unique.push(value);
        }
    });
    *values = unique;
}
