//! Applies address-method intents through the netdev operations.

use core::fmt::Display;
use core::net::IpAddr;

use addrconf::{AddressConfig, Intent};
use netdev::Ops;

#[cfg(feature = "ipv4")]
use super::V4Notice;
#[cfg(feature = "ipv6")]
use super::V6Notice;
use super::{Interface, notice};

#[derive(Clone, Copy)]
enum Family {
    #[cfg(feature = "ipv4")]
    V4,
    #[cfg(feature = "ipv6")]
    V6,
}

pub(super) async fn apply<N: Ops>(interface: &mut Interface<N>, intent: Intent) {
    match intent {
        Intent::Configure(config) => ensure_address(interface, config).await,
        Intent::Deconfigure(config) => remove_address(interface, config).await,
        Intent::EnsureDefaultRoute(gateway) => ensure_route(interface, gateway).await,
        Intent::RemoveDefaultRoute(gateway) => remove_route(interface, gateway).await,
    }
}

async fn ensure_address<N: Ops>(interface: &mut Interface<N>, config: AddressConfig) {
    let index = interface.link.index;
    let prefix_len = config.prefix_len;
    match config.address {
        #[cfg(feature = "ipv4")]
        IpAddr::V4(address) => {
            let result = interface.ops.ensure_ipv4(index, address, prefix_len).await;
            report(interface, result, "ensure IPv4", Family::V4);
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(address) => {
            let result = interface.ops.ensure_ipv6(index, address, prefix_len).await;
            report(interface, result, "ensure IPv6", Family::V6);
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
            report(interface, result, "remove IPv4", Family::V4);
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(address) => {
            let result = interface.ops.remove_ipv6(index, address).await;
            report(interface, result, "remove IPv6", Family::V6);
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
            report(interface, result, "ensure default route", Family::V4);
        }
        #[cfg(feature = "ipv6")]
        IpAddr::V6(gateway) => {
            let result = interface.ops.ensure_default_route_v6(gateway).await;
            report(interface, result, "ensure default route", Family::V6);
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
            report(interface, result, "remove default route", Family::V6);
        }
    }
}

#[cfg(not(feature = "ipv6"))]
async fn remove_route<N: Ops>(_: &mut Interface<N>, _: IpAddr) {}

fn report<N: Ops, T, E: Display>(
    interface: &Interface<N>,
    result: Result<T, E>,
    action: &str,
    family: Family,
) {
    if let Err(error) = result {
        let warning = alloc::format!("{action} on {}: {error}", interface.link.name);
        match family {
            #[cfg(feature = "ipv4")]
            Family::V4 => notice::notify_v4(interface, V4Notice::Warning(warning)),
            #[cfg(feature = "ipv6")]
            Family::V6 => notice::notify_v6(interface, V6Notice::Warning(warning)),
        }
    }
}
