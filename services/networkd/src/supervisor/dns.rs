//! DNS state flushed from the L3 owner's configuration, with config fallback.

use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use linkctl::interface::{Handle, Snapshot};
#[cfg(feature = "ipv4")]
use linkctl::view::V4State;
#[cfg(feature = "ipv6")]
use linkctl::view::V6State;
use netdev::Ops;

use super::{NetworkSupervisor, owner};

/// Flushes DNS configuration based on the L3 owner's state with fallback.
pub(super) fn flush<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let Some(owner) = owner::l3_owner(
        &supervisor.state,
        &supervisor.interfaces,
        &supervisor.owners,
    ) else {
        return;
    };
    let Some(handle) = supervisor.interfaces.get(owner) else {
        return;
    };
    let (v4, v6) = collect(supervisor, handle);
    if supervisor.dns.is_unchanged(&v4, &v6) {
        return;
    }
    apply(supervisor, v4, v6);
}

fn collect<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    handle: &Handle,
) -> (Vec<Ipv4Addr>, Vec<Ipv6Addr>) {
    let snapshot = handle.snapshot_rx.borrow();

    (
        collect_v4(supervisor, &snapshot),
        collect_v6(supervisor, &snapshot),
    )
}

#[cfg(feature = "ipv4")]
fn collect_v4<N: Ops>(supervisor: &NetworkSupervisor<N>, snapshot: &Snapshot) -> Vec<Ipv4Addr> {
    let mut dns = Vec::new();
    if let Some(state) = snapshot.v4.as_ref() {
        match *state {
            V4State::Static(ref config) => dns.clone_from(&config.dns),
            V4State::Dhcp(ref view) => dns.clone_from(&view.lease.dns_servers),
        }
    }
    if dns.is_empty() {
        dns = supervisor.config.ipv4_dns().collect();
    }

    dns
}

#[cfg(not(feature = "ipv4"))]
fn collect_v4<N: Ops>(_: &NetworkSupervisor<N>, _: &Snapshot) -> Vec<Ipv4Addr> {
    Vec::new()
}

#[cfg(feature = "ipv6")]
fn collect_v6<N: Ops>(supervisor: &NetworkSupervisor<N>, snapshot: &Snapshot) -> Vec<Ipv6Addr> {
    let mut dns = Vec::new();
    if let Some(state) = snapshot.v6.as_ref() {
        match *state {
            V6State::Static(ref config) => dns.clone_from(&config.dns),
            V6State::Slaac(ref view) => dns.clone_from(&view.dns),
        }
    }
    if dns.is_empty() {
        dns = supervisor.config.ipv6_dns().collect();
    }

    dns
}

#[cfg(not(feature = "ipv6"))]
fn collect_v6<N: Ops>(_: &NetworkSupervisor<N>, _: &Snapshot) -> Vec<Ipv6Addr> {
    Vec::new()
}

fn apply<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    ipv4_dns: Vec<Ipv4Addr>,
    ipv6_dns: Vec<Ipv6Addr>,
) {
    if let Err(e) = supervisor.dns.update(ipv4_dns, ipv6_dns) {
        kmsg::warn!("Failed to write DNS: {e}");
    }
}
