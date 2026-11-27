//! Lease to provisioning intent translation.

use core::net::IpAddr;

use ip_method::{AddressConfig, Event, Intent};

use crate::Dhcp;
use crate::effects::Action;

pub(super) fn configure(method: &mut Dhcp) {
    let Some(lease) = method.lease.as_ref() else {
        return;
    };
    let config = AddressConfig {
        address: IpAddr::V4(lease.assigned_ip),
        prefix_len: lease.prefix_len,
    };
    method
        .queue
        .push_back(Action::Event(Event::Intent(Intent::Configure(config))));
    if let Some(gateway) = lease.gateway {
        let intent = Intent::EnsureDefaultRoute(IpAddr::V4(gateway));
        method.queue.push_back(Action::Event(Event::Intent(intent)));
    }
}

pub(super) fn deconfigure(method: &mut Dhcp) {
    let Some(lease) = method.lease.as_ref() else {
        return;
    };
    let config = AddressConfig {
        address: IpAddr::V4(lease.assigned_ip),
        prefix_len: lease.prefix_len,
    };
    method
        .queue
        .push_back(Action::Event(Event::Intent(Intent::Deconfigure(config))));
    if let Some(gateway) = lease.gateway {
        let intent = Intent::RemoveDefaultRoute(IpAddr::V4(gateway));
        method.queue.push_back(Action::Event(Event::Intent(intent)));
    }
}
