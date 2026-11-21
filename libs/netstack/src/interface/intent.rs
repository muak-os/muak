//! Applies address-method intents through the netdev operations.

use core::fmt::Display;
use core::net::IpAddr;

use netdev::Ops;

use super::Interface;
use crate::provision;
use crate::provision::AddressConfig;

pub(super) async fn apply<N: Ops>(interface: &mut Interface<N>, intent: provision::Intent) {
    match intent {
        provision::Intent::Configure(config) => ensure_address(interface, config).await,
        provision::Intent::Deconfigure(config) => remove_address(interface, config).await,
        provision::Intent::EnsureDefaultRoute(gateway) => ensure_route(interface, gateway).await,
        provision::Intent::RemoveDefaultRoute(gateway) => remove_route(interface, gateway).await,
    }
}

async fn ensure_address<N: Ops>(interface: &mut Interface<N>, config: AddressConfig) {
    let index = interface.link.index;
    let prefix_len = config.prefix_len;
    match config.address {
        #[cfg(feature = "ipv4")]
        IpAddr::V4(address) => {
            let result = interface.ops.ensure_ipv4(index, address, prefix_len).await;
            report(interface, result, "ensure IPv4");
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(address) => {
            let result = interface.ops.ensure_ipv6(index, address, prefix_len).await;
            report(interface, result, "ensure IPv6");
        }
        #[cfg(not(all(feature = "ipv4", feature = "ipv6")))]
        _ => {}
    }
}

async fn remove_address<N: Ops>(interface: &mut Interface<N>, config: AddressConfig) {
    let index = interface.link.index;
    match config.address {
        #[cfg(feature = "ipv4")]
        IpAddr::V4(address) => {
            let result = interface.ops.remove_ipv4(index, address).await;
            report(interface, result, "remove IPv4");
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(address) => {
            let result = interface.ops.remove_ipv6(index, address).await;
            report(interface, result, "remove IPv6");
        }
        #[cfg(not(all(feature = "ipv4", feature = "ipv6")))]
        _ => {}
    }
}

async fn ensure_route<N: Ops>(interface: &mut Interface<N>, gateway: IpAddr) {
    match gateway {
        #[cfg(feature = "ipv4")]
        IpAddr::V4(gateway) => {
            let result = interface.ops.ensure_default_route(gateway).await;
            report(interface, result, "ensure default route");
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(gateway) => {
            let result = interface.ops.ensure_default_route_v6(gateway).await;
            report(interface, result, "ensure default route");
        }
        #[cfg(not(all(feature = "ipv4", feature = "ipv6")))]
        _ => {}
    }
}

#[cfg(feature = "ipv6")]
async fn remove_route<N: Ops>(interface: &mut Interface<N>, gateway: IpAddr) {
    match gateway {
        IpAddr::V4(_) => {}
        IpAddr::V6(gateway) => {
            let result = interface.ops.remove_default_route_v6(gateway).await;
            report(interface, result, "remove default route");
        }
    }
}

#[cfg(not(feature = "ipv6"))]
async fn remove_route<N: Ops>(_: &mut Interface<N>, _: IpAddr) {}

fn report<N: Ops, T, E: Display>(interface: &Interface<N>, result: Result<T, E>, action: &str) {
    if let Err(error) = result {
        kmsg::warn!("netstack: {action} on {}: {error}", interface.link.name);
    }
}
