//! Resolver aggregation from the layer 3 owner, with config fallback.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::actions::Action;
use crate::{Supervisor, owner};

trait Family: PartialEq + Sized {
    type View;

    fn enabled() -> bool;
    fn from_ip(address: IpAddr) -> Option<Self>;
    fn view(snapshot: &iface::Snapshot) -> Option<&Self::View>;
    fn view_dns(view: &Self::View) -> Vec<Self>;
}

impl Family for Ipv4Addr {
    type View = iface::V4View;

    fn enabled() -> bool {
        cfg!(feature = "ipv4")
    }

    fn from_ip(address: IpAddr) -> Option<Self> {
        match address {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        }
    }

    fn view(snapshot: &iface::Snapshot) -> Option<&Self::View> {
        snapshot.v4.as_ref()
    }

    fn view_dns(view: &Self::View) -> Vec<Self> {
        match *view {
            iface::V4View::Static(ref view) => view.dns().filter_map(Self::from_ip).collect(),
            #[cfg(feature = "ipv4")]
            iface::V4View::Dhcp(ref view) => view.lease.dns_servers.clone(),
        }
    }
}

impl Family for Ipv6Addr {
    type View = iface::V6View;

    fn enabled() -> bool {
        cfg!(feature = "ipv6")
    }

    fn from_ip(address: IpAddr) -> Option<Self> {
        match address {
            IpAddr::V6(address) => Some(address),
            IpAddr::V4(_) => None,
        }
    }

    fn view(snapshot: &iface::Snapshot) -> Option<&Self::View> {
        snapshot.v6.as_ref()
    }

    fn view_dns(view: &Self::View) -> Vec<Self> {
        match *view {
            iface::V6View::Static(ref view) => view.dns().filter_map(Self::from_ip).collect(),
            #[cfg(feature = "ipv6")]
            iface::V6View::Slaac(ref view) => view.dns.clone(),
        }
    }
}

/// Emits a resolver write when the desired nameservers changed.
pub(crate) fn flush(supervisor: &mut Supervisor) {
    let Some(owner) = owner::owner(supervisor).cloned() else {
        return;
    };
    let v4 = desired::<Ipv4Addr>(supervisor, &owner);
    let v6 = desired::<Ipv6Addr>(supervisor, &owner);
    if v4 == supervisor.resolver_v4 && v6 == supervisor.resolver_v6 {
        return;
    }

    supervisor.resolver_v4 = v4;
    supervisor.resolver_v6 = v6;
    supervisor.queue.push_back(Action::WriteResolver);
}

fn desired<F: Family>(supervisor: &Supervisor, owner: &netmodel::Name) -> Vec<F> {
    if !F::enabled() {
        return Vec::new();
    }

    let mut dns = supervisor
        .snapshots
        .get(owner)
        .and_then(F::view)
        .map_or_default(F::view_dns);
    if dns.is_empty() {
        dns = supervisor
            .config
            .dns
            .iter()
            .filter_map(|address| F::from_ip(*address))
            .collect();
    }
    dedup(&mut dns);

    dns
}

fn dedup<T: PartialEq>(values: &mut Vec<T>) {
    let mut unique: Vec<T> = Vec::with_capacity(values.len());
    values.drain(..).for_each(|value| {
        if !unique.contains(&value) {
            unique.push(value);
        }
    });
    *values = unique;
}
