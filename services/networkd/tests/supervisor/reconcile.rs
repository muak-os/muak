//! Integration tests for supervisor reconciliation.

use core::time::Duration;

use config::system::network;
use netlink::link::ifname::Name;
use netlink_monitor::Event;
use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::sleep;

use super::*;

/// Returns a named static IPv4 test config for supervisor reconciliation.
fn config_static_named() -> network::Config {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(192, 168, 10, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(192, 168, 10, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    cfg
}

/// Returns a named static IPv6 test config for supervisor reconciliation.
#[cfg(feature = "ipv6")]
fn config_static_ipv6_named() -> network::Config {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
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
    cfg
}

/// Returns a DHCP test config for supervisor reconciliation.
fn config_dhcp_named() -> network::Config {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: true,
            addresses: vec![],
            gateway: None,
        }),
        ipv6: None,
        bridge: None,
    });
    cfg
}

/// Returns a bridge config whose L3 intent is declared on the bridge entry.
fn config_bridge_l3_on_bridge() -> network::Config {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "eth0".to_owned(),
        kind: network::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: None,
        bridge: None,
    });
    cfg.interfaces.push(network::InterfaceConfig {
        name: "br0".to_owned(),
        kind: network::InterfaceKind::Bridge,
        ipv4: Some(network::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![network::Cidr4 {
                address: core::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: Some(network::BridgeConfig {
            port: vec!["eth0".to_owned()],
            stp: false,
        }),
    });
    cfg
}

/// Returns a bridge config that targets an unresolved named port.
fn config_bridge_missing_port() -> network::Config {
    let mut cfg = network::Config::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(network::InterfaceConfig {
        name: "br0".to_owned(),
        kind: network::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(network::BridgeConfig {
            port: vec!["eth9".to_owned()],
            stp: false,
        }),
    });
    cfg
}

#[tokio::test]
async fn reconcile_reapplies_static_ipv4_after_drift() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_static_named(),
        Resolver::default(),
    );

    handle.initialize_with_retry().await.expect("init failed");

    mock.remove_ipv4(idx, core::net::Ipv4Addr::new(192, 168, 10, 2))
        .await
        .expect("remove failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    let addrs = ipv4_addrs(&mock, idx);
    assert!(
        addrs.contains(&(core::net::Ipv4Addr::new(192, 168, 10, 2), 24)),
        "expected 192.168.10.2/24 in {addrs:?}"
    );
}

#[tokio::test]
async fn reconcile_before_initialization_is_a_noop() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_static_named(),
        Resolver::default(),
    );

    // ACT
    handle.reconcile().await.expect("reconcile failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(
        ipv4_addrs(&mock, idx).is_empty(),
        "reconcile should do nothing before initialization"
    );
}

#[cfg(feature = "ipv6")]
#[tokio::test]
async fn reconcile_reapplies_static_ipv6_after_drift() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_static_ipv6_named(),
        Resolver::default(),
    );

    handle.initialize_with_retry().await.expect("init failed");

    mock.remove_ipv6(idx, "2001:db8::2".parse().expect("valid ipv6"))
        .await
        .expect("remove failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    let addrs = ipv6_addrs(&mock, idx);
    assert!(
        addrs.contains(&("2001:db8::2".parse().expect("valid ipv6"), 64)),
        "expected 2001:db8::2/64 in {addrs:?}"
    );
    assert!(
        has_default_route_v6(&mock, "2001:db8::1".parse().expect("valid ipv6")),
        "expected IPv6 default route to be restored"
    );
}

#[tokio::test]
async fn reconcile_with_dhcp_config_completes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_dhcp_named(), Resolver::default());

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    let result = handle.reconcile().await;

    // ASSERT
    assert!(result.is_ok(), "DHCP reconcile should complete");
}

#[tokio::test]
async fn reconcile_skips_bridge_when_port_is_missing() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_missing_port(),
        Resolver::default(),
    );

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(!has_link(&mock, "br0"), "bridge should not be created");
}

#[tokio::test]
async fn reconcile_named_missing_interface_is_non_fatal() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config_static_named(), Resolver::default());

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    let result = handle.reconcile().await;

    // ASSERT
    assert!(
        result.is_ok(),
        "reconcile should log and continue on missing interface"
    );
}

#[tokio::test]
async fn reconcile_recreates_missing_bridge() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_l3_on_bridge(),
        Resolver::default(),
    );
    handle.initialize_with_retry().await.expect("init failed");
    sleep(Duration::from_millis(100)).await;
    let br0_index = link_index(&mock, "br0").expect("bridge should exist");
    let ensure_calls = mock.ensure_bridge_calls();
    assert!(ensure_calls >= 1, "provisioning should create the bridge");

    mock.remove_link("br0");
    event_tx
        .send(Event::Deleted {
            name: Name::new("br0").expect("valid bridge name"),
            index: br0_index,
        })
        .await
        .expect("send failed");
    sleep(Duration::from_millis(100)).await;

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert_eq!(
        mock.ensure_bridge_calls(),
        ensure_calls + 1,
        "missing bridge should be recreated"
    );
    let new_index = link_index(&mock, "br0").expect("bridge should be recreated");
    assert_ne!(new_index, br0_index, "recreated bridge needs a new index");
    assert_eq!(
        master_index(&mock, "eth0"),
        Some(new_index),
        "port should be attached to the recreated bridge"
    );
    assert!(
        ipv4_addrs(&mock, new_index).contains(&(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)),
        "L3 intent should be reapplied to the recreated bridge"
    );
}

#[tokio::test]
async fn reconcile_reapplies_bridge_own_ipv4() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_l3_on_bridge(),
        Resolver::default(),
    );
    handle.initialize_with_retry().await.expect("init failed");
    sleep(Duration::from_millis(100)).await;
    let br0_index = link_index(&mock, "br0").expect("bridge should exist");

    mock.remove_ipv4(br0_index, core::net::Ipv4Addr::new(10, 0, 0, 2))
        .await
        .expect("remove failed");
    event_tx
        .send(Event::Deleted {
            name: Name::new("br0").expect("valid bridge name"),
            index: br0_index,
        })
        .await
        .expect("send failed");
    sleep(Duration::from_millis(100)).await;

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(
        ipv4_addrs(&mock, br0_index).contains(&(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)),
        "bridge entry intent should be reapplied to the bridge"
    );
}

#[cfg(feature = "ipv6")]
#[tokio::test]
async fn reconcile_keeps_ipv6_on_the_bridge() {
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
    handle.initialize_with_retry().await.expect("init failed");
    sleep(Duration::from_millis(100)).await;
    let br0_index = link_index(&mock, "br0").expect("bridge should exist");

    mock.remove_ipv6(br0_index, "2001:db8::2".parse().expect("valid ipv6"))
        .await
        .expect("remove failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(
        ipv6_addrs(&mock, br0_index).contains(&("2001:db8::2".parse().expect("valid ipv6"), 64)),
        "reconcile must keep static IPv6 on the bridge"
    );
    assert!(
        ipv6_addrs(&mock, eth0_index).is_empty(),
        "reconcile must not move IPv6 onto the port"
    );
}

#[tokio::test]
async fn reconcile_repoints_a_configured_bridge() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        event_rx,
        config_bridge_l3_on_bridge(),
        Resolver::default(),
    );
    handle.initialize_with_retry().await.expect("init failed");
    sleep(Duration::from_millis(100)).await;
    let br0_index = link_index(&mock, "br0").expect("bridge should exist");
    let ensure_calls = mock.ensure_bridge_calls();
    let owner = handle.status().await.expect("status query failed").owner;
    assert_eq!(
        owner,
        Some(Name::new("br0").expect("valid bridge name")),
        "the bridge should own L3 after initialization"
    );
    assert!(
        ipv4_addrs(&mock, br0_index).contains(&(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)),
        "the bridge should have its static IPv4 after initialization"
    );

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert_eq!(
        mock.ensure_bridge_calls(),
        ensure_calls + 1,
        "reconcile must re-point a configured bridge"
    );
    assert_eq!(
        link_index(&mock, "br0"),
        Some(br0_index),
        "the kernel bridge index is unchanged"
    );
    assert_eq!(
        handle.status().await.expect("status query failed").owner,
        Some(Name::new("br0").expect("valid bridge name")),
        "L3 ownership must stay on the bridge"
    );
    assert!(
        ipv4_addrs(&mock, br0_index).contains(&(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)),
        "the bridge must keep its static IPv4"
    );
}
