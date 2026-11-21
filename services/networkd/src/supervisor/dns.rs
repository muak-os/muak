//! DNS state flushed from the L3 owner's configuration, with config fallback.

use core::net::{Ipv4Addr, Ipv6Addr};

use netdev::Ops;

use super::{ActorHandle, NetworkSupervisor, failover};

/// Flushes DNS configuration based on the L3 owner's state with fallback.
pub(super) fn flush<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let Some(owner) = failover::l3_owner(&supervisor.state, &supervisor.interfaces) else {
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
    handle: &ActorHandle,
) -> (Vec<Ipv4Addr>, Vec<Ipv6Addr>) {
    let snap = handle.state_rx.borrow();
    let ipv4_dns = snap
        .ip
        .as_ref()
        .filter(|ip| !ip.dns.is_empty())
        .map_or_else(
            || supervisor.config.ipv4_dns().collect(),
            |ip| ip.dns.clone(),
        );
    let ipv6_dns = snap
        .ipv6
        .as_ref()
        .filter(|ip| !ip.dns.is_empty())
        .map_or_else(
            || supervisor.config.ipv6_dns().collect(),
            |ip| ip.dns.clone(),
        );

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
