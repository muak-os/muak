//! Integration tests for the network supervisor.

extern crate alloc;

#[cfg(test)]
mod supervisor {
    pub(super) use networkd::supervisor;

    pub(super) use super::*;

    mod discovery;
    mod dispatch;
    mod dns;
    mod failover;
    mod lifecycle;
    mod provision;
    mod reconcile;
}

mod fixtures;

use alloc::sync::Arc;

use netlib::address::Ops as _;

use self::fixtures::netlink::MockNetlinkOps;

fn make_config() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_owned(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

/// Returns a config with a static IPv4 port and a bridge over it.
fn config_bridge() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_owned(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_owned(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
            port: vec!["auto".to_owned()],
            stp: true,
        }),
    });

    Arc::new(cfg)
}

fn ipv4_addrs(
    mock: &MockNetlinkOps,
    index: u32,
) -> std::collections::HashSet<(core::net::Ipv4Addr, u8)> {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ipv4_addrs
        .get(&index)
        .cloned()
        .unwrap_or_default()
}

fn ipv6_addrs(
    mock: &MockNetlinkOps,
    index: u32,
) -> std::collections::HashSet<(core::net::Ipv6Addr, u8)> {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ipv6_addrs
        .get(&index)
        .cloned()
        .unwrap_or_default()
}

fn has_link(mock: &MockNetlinkOps, name: &str) -> bool {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .links
        .contains_key(name)
}

fn has_default_route_v6(mock: &MockNetlinkOps, gateway: core::net::Ipv6Addr) -> bool {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .default_routes_v6
        .contains(&gateway)
}

/// Returns the kernel index of a mock link, if present.
fn link_index(mock: &MockNetlinkOps, name: &str) -> Option<u32> {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .links
        .get(name)
        .map(|link| link.index)
}

/// Returns the bridge master index of a mock link, if any.
fn master_index(mock: &MockNetlinkOps, name: &str) -> Option<u32> {
    mock.state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .links
        .get(name)
        .and_then(|link| link.master_index)
}
