//! Lease to provisioning intent translation.

use core::net::IpAddr;

use addrconf::{AddressConfig, Event, Intent};

use super::{Dhcp, Exchange};

pub(super) fn configure<E: Exchange>(method: &mut Dhcp<E>) {
    let Some(lease) = method.lease.as_ref() else {
        return;
    };
    let config = AddressConfig {
        address: IpAddr::V4(lease.assigned_ip),
        prefix_len: lease.prefix_len,
    };
    method
        .queue
        .push_back(Event::Intent(Intent::Configure(config)));
    if let Some(gateway) = lease.gateway {
        method
            .queue
            .push_back(Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V4(
                gateway,
            ))));
    }
}

pub(super) fn deconfigure<E: Exchange>(method: &mut Dhcp<E>) {
    let Some(lease) = method.lease.as_ref() else {
        return;
    };
    let config = AddressConfig {
        address: IpAddr::V4(lease.assigned_ip),
        prefix_len: lease.prefix_len,
    };
    method
        .queue
        .push_back(Event::Intent(Intent::Deconfigure(config)));
    if let Some(gateway) = lease.gateway {
        method
            .queue
            .push_back(Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V4(
                gateway,
            ))));
    }
}
