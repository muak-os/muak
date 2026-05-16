//! Integration tests for the network supervisor.

#[path = "supervisor/discovery.rs"]
mod discovery;
#[path = "supervisor/dispatch.rs"]
mod dispatch;
#[path = "supervisor/dns.rs"]
mod dns;
#[path = "supervisor/failover.rs"]
mod failover;
mod fixtures;
#[path = "supervisor/lifecycle.rs"]
mod lifecycle;
#[path = "supervisor/provision.rs"]
mod provision;
#[path = "supervisor/reconcile.rs"]
mod reconcile;

use std::sync::Arc;

use netlib::address::AddressOps;
use networkd::supervisor;

use self::fixtures::netlink::MockNetlinkOps;

fn make_config() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: std::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(std::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn ipv4_addrs(
    mock: &MockNetlinkOps,
    index: u32,
) -> std::collections::HashSet<(std::net::Ipv4Addr, u8)> {
    mock.state
        .lock()
        .expect("lock")
        .ipv4_addrs
        .get(&index)
        .cloned()
        .unwrap_or_default()
}

fn ipv6_addrs(
    mock: &MockNetlinkOps,
    index: u32,
) -> std::collections::HashSet<(std::net::Ipv6Addr, u8)> {
    mock.state
        .lock()
        .expect("lock")
        .ipv6_addrs
        .get(&index)
        .cloned()
        .unwrap_or_default()
}

fn has_link(mock: &MockNetlinkOps, name: &str) -> bool {
    mock.state.lock().expect("lock").links.contains_key(name)
}

fn has_default_route_v6(mock: &MockNetlinkOps, gateway: std::net::Ipv6Addr) -> bool {
    mock.state
        .lock()
        .expect("lock")
        .default_routes_v6
        .contains(&gateway)
}
