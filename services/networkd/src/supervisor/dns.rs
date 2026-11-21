//! DNS state flushed from the L3 owner's configuration, with config fallback.

use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr};

use netdev::Ops;
use netstack::interface::Handle;
use netstack::view::V4State;
#[cfg(feature = "ipv6")]
use netstack::view::V6State;

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
    let mut ipv4_dns = Vec::new();
    if let Some(state) = snapshot.v4.as_ref() {
        match *state {
            V4State::Static(ref config) => ipv4_dns.clone_from(&config.dns),
            V4State::Dhcp { ref lease, .. } => ipv4_dns.clone_from(&lease.dns_servers),
        }
    }
    if ipv4_dns.is_empty() {
        ipv4_dns = supervisor.config.ipv4_dns().collect();
    }

    #[cfg(feature = "ipv6")]
    let mut ipv6_dns = Vec::new();
    #[cfg(feature = "ipv6")]
    if let Some(state) = snapshot.v6.as_ref() {
        match *state {
            V6State::Static(ref config) => ipv6_dns.clone_from(&config.dns),
            V6State::Slaac { ref dns, .. } => ipv6_dns.clone_from(dns),
        }
    }
    #[cfg(feature = "ipv6")]
    if ipv6_dns.is_empty() {
        ipv6_dns = supervisor.config.ipv6_dns().collect();
    }
    #[cfg(not(feature = "ipv6"))]
    let ipv6_dns = Vec::new();

    (ipv4_dns, ipv6_dns)
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
