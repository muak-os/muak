//! Applies address-method intents through the netdev operations.

use core::fmt::Display;
use core::net::IpAddr;

use addrconf::{AddressConfig, Intent};
use netdev::Ops;

use super::{Interface, Notice};
use crate::family::Family;
use crate::view;

#[derive(Clone, Copy)]
enum Target {
    V4,
    V6,
}

pub(super) async fn apply<N, V4, V6>(interface: &mut Interface<N, V4, V6>, intent: Intent)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match intent {
        Intent::Configure(config) => ensure_address(interface, config).await,
        Intent::Deconfigure(config) => remove_address(interface, config).await,
        Intent::EnsureDefaultRoute(gateway) => ensure_route(interface, gateway).await,
        Intent::RemoveDefaultRoute(gateway) => remove_route(interface, gateway).await,
    }
}

async fn ensure_address<N, V4, V6>(interface: &mut Interface<N, V4, V6>, config: AddressConfig)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let index = interface.link.index;
    let prefix_len = config.prefix_len;
    match config.address {
        IpAddr::V4(address) => {
            let result = interface.ops.ensure_ipv4(index, address, prefix_len).await;
            report(interface, result, "ensure IPv4", Target::V4);
        }
        IpAddr::V6(address) => {
            let result = interface.ops.ensure_ipv6(index, address, prefix_len).await;
            report(interface, result, "ensure IPv6", Target::V6);
        }
    }
}

async fn remove_address<N, V4, V6>(interface: &mut Interface<N, V4, V6>, config: AddressConfig)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let index = interface.link.index;
    match config.address {
        IpAddr::V4(address) => {
            let result = interface.ops.remove_ipv4(index, address).await;
            report(interface, result, "remove IPv4", Target::V4);
        }
        IpAddr::V6(address) => {
            let result = interface.ops.remove_ipv6(index, address).await;
            report(interface, result, "remove IPv6", Target::V6);
        }
    }
}

async fn ensure_route<N, V4, V6>(interface: &mut Interface<N, V4, V6>, gateway: IpAddr)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match gateway {
        IpAddr::V4(gateway) => {
            let result = interface.ops.ensure_default_route(gateway).await;
            report(interface, result, "ensure default route", Target::V4);
        }
        IpAddr::V6(gateway) => {
            let result = interface.ops.ensure_default_route_v6(gateway).await;
            report(interface, result, "ensure default route", Target::V6);
        }
    }
}

async fn remove_route<N, V4, V6>(interface: &mut Interface<N, V4, V6>, gateway: IpAddr)
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    match gateway {
        IpAddr::V4(_) => {}
        IpAddr::V6(gateway) => {
            let result = interface.ops.remove_default_route_v6(gateway).await;
            report(interface, result, "remove default route", Target::V6);
        }
    }
}

fn report<N, V4, V6, T, E: Display>(
    interface: &Interface<N, V4, V6>,
    result: Result<T, E>,
    action: &str,
    target: Target,
) where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    if let Err(error) = result {
        let warning = alloc::format!("{action} on {}: {error}", interface.link.name);
        match target {
            Target::V4 => interface.v4.notify(Notice::Warning(warning)),
            Target::V6 => interface.v6.notify(Notice::Warning(warning)),
        }
    }
}
