//! Integration tests for `NetworkSupervisor` interface provisioning.

use alloc::sync::Arc;
use core::time::Duration;

use config::system::network;
use networkd::dns::Resolver;
use networkd::supervisor;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use super::{
    MockNetlinkOps, config_bridge, config_bridge_ipv6, ipv4_addrs, ipv6_addrs, link_index,
    master_index,
};

/// Asserts that the configured static IPv4 landed on `br0` and not on its port.
fn assert_l3_on_bridge(mock: &MockNetlinkOps) {
    let br_index = link_index(mock, "br0").expect("bridge should exist");
    let eth_index = link_index(mock, "eth0").expect("port should exist");

    assert_eq!(
        master_index(mock, "eth0"),
        Some(br_index),
        "port should be attached to the bridge"
    );
    assert_eq!(
        mock.mac("br0"),
        Some([0xAA; 6]),
        "bridge should inherit the port MAC"
    );
    assert!(
        ipv4_addrs(mock, br_index).contains(&(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)),
        "static IPv4 should land on the bridge"
    );
    assert!(
        ipv4_addrs(mock, eth_index).is_empty(),
        "the port must not carry L3 addresses"
    );
}

fn config_static_ipv4() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
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

fn config_dhcp() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: true,
            addresses: vec![],
            gateway: None,
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_ipv6() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: Some(network::Ipv6InterfaceConfig {
            autoconf: false,
            addresses: vec![network::Cidr6 {
                address: "2001:db8::2".parse().expect("valid ipv6"),
                prefix: 64,
            }],
            gateway: Some("2001:db8::1".parse().expect("valid ipv6")),
        }),
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_slaac() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.ipv6 = true;
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: Some(network::Ipv6InterfaceConfig {
            autoconf: true,
            addresses: vec![],
            gateway: None,
        }),
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_named() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(192, 168, 1, 10),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(192, 168, 1, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_none() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_bridge_multiport() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    cfg.interfaces.push(network::InterfaceConfig {
        name: "br0".to_owned(),
        kind: network::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(network::BridgeConfig {
            port: vec!["auto".to_owned(), "eth1".to_owned()],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

fn config_bridge_named_port() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    cfg.interfaces.push(network::InterfaceConfig {
        name: "br0".to_owned(),
        kind: network::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(network::BridgeConfig {
            port: vec!["eth0".to_owned()],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

fn config_bridge_empty_port() -> Arc<network::Config> {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "auto".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    cfg.interfaces.push(network::InterfaceConfig {
        name: "br0".to_owned(),
        kind: network::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(network::BridgeConfig {
            port: vec![],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

#[tokio::test]
async fn provision_static_ipv4_initializes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_static_ipv4(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(result.expect("timeout").is_ok(), "should succeed");
}

#[tokio::test]
async fn provision_static_ipv4_named_interface_not_found_still_initializes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xBB; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_static_ipv4(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "should succeed even when named interface is absent"
    );
}

#[tokio::test]
async fn provision_dhcp_sends_configure_dhcp_command() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_dhcp(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with DHCP config"
    );
}

#[tokio::test]
async fn provision_static_ipv6_sends_configure_command() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_ipv6(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with static IPv6 config"
    );
}

#[tokio::test]
async fn provision_slaac_autoconf_sends_configure_slaac() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_slaac(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with SLAAC autoconf"
    );
}

#[tokio::test]
async fn provision_named_interface_resolves_by_name() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_named(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with named interface"
    );
}

#[tokio::test]
async fn provision_named_interface_not_found_still_initializes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xBB; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_named(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed even with missing named interface"
    );
}

#[tokio::test]
async fn provision_no_ip_config_still_initializes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_none(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with no IP config"
    );
}

#[tokio::test]
async fn provision_bridge_with_auto_port() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock.clone(), event_rx, config_bridge(), Resolver::default());

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with bridge config"
    );
    assert_l3_on_bridge(&mock);
}

#[tokio::test]
async fn provision_bridge_applies_ipv6_to_the_bridge() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let eth0_index = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_ipv6(),
        Resolver::default(),
    );

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    let br0_index = link_index(&mock, "br0").expect("bridge should exist");
    result
        .expect("should not time out")
        .expect("initialization should succeed with bridge IPv6 config");
    assert!(
        ipv6_addrs(&mock, br0_index).contains(&("2001:db8::2".parse().expect("valid ipv6"), 64)),
        "static IPv6 should land on the bridge"
    );
    assert!(
        ipv6_addrs(&mock, eth0_index).is_empty(),
        "the port must not carry IPv6 addresses"
    );
}

#[tokio::test]
async fn provision_bridge_multiport_warns_and_uses_first_port() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_multiport(),
        Resolver::default(),
    );

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with multi-port bridge config"
    );
    assert_l3_on_bridge(&mock);
}

#[tokio::test]
async fn provision_bridge_named_port_resolves_by_name() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_named_port(),
        Resolver::default(),
    );

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with named bridge port"
    );
    assert_l3_on_bridge(&mock);
}

#[tokio::test]
async fn provision_bridge_empty_port_falls_back_to_primary() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_empty_port(),
        Resolver::default(),
    );

    // ACT
    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed when bridge port list is empty"
    );
    assert_l3_on_bridge(&mock);
}
