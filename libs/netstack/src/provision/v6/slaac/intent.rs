//! Provisioning intents and DNS state derived from router advertisements.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv6Addr};
use std::time::Instant;

use ::slaac::icmpv6::types::RouterAdvertisement;

use super::Slaac;
use super::state::ManagedDns;
use crate::provision::{AddressConfig, Intent};
use crate::transport::icmpv6::Transport;

pub(super) const fn configure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Configure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

pub(super) const fn deconfigure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Deconfigure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

pub(super) fn resolve_dns<T: Transport>(
    method: &Slaac<T>,
    ra: &RouterAdvertisement,
    now: Instant,
) -> Vec<ManagedDns> {
    if ra.dns_servers.is_empty() {
        method.dns_fallback(now)
    } else {
        ra.dns_servers
            .iter()
            .map(|&server| ManagedDns::new(server, u64::from(ra.dns_lifetime), now))
            .collect()
    }
}
