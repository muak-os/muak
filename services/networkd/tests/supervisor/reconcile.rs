//! Integration tests for supervisor reconciliation.

use std::sync::Arc;
use std::time::Duration;

use super::*;

/// Returns a named static IPv4 test config for supervisor reconciliation.
fn config_static_named() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "eth0".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: std::net::Ipv4Addr::new(192, 168, 10, 2),
                prefix: 24,
            }],
            gateway: Some(std::net::Ipv4Addr::new(192, 168, 10, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

/// Returns a named static IPv6 test config for supervisor reconciliation.
fn config_static_ipv6_named() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "eth0".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: Some(config::Ipv6InterfaceConfig {
            autoconf: false,
            addresses: vec![config::Cidr6 {
                address: "2001:db8::2".parse().expect("valid ipv6"),
                prefix: 64,
            }],
            gateway: Some("2001:db8::1".parse().expect("valid ipv6")),
        }),
        bridge: None,
    });
    Arc::new(cfg)
}

/// Returns a DHCP test config for supervisor reconciliation.
fn config_dhcp_named() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "eth0".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: true,
            addresses: vec![],
            gateway: None,
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

/// Returns a bridge config that targets an unresolved named port.
fn config_bridge_missing_port() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_string(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
            port: vec!["eth9".to_string()],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

/// Verifies supervisor reconcile restores drifted static IPv4 state.
#[tokio::test]
async fn reconcile_reapplies_static_ipv4_after_drift() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        Some(event_rx),
        config_static_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    handle.initialize_with_retry().await.expect("init failed");

    mock.remove_ipv4(idx, std::net::Ipv4Addr::new(192, 168, 10, 2))
        .await
        .expect("remove failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    let addrs = ipv4_addrs(&mock, idx);
    assert!(
        addrs.contains(&(std::net::Ipv4Addr::new(192, 168, 10, 2), 24)),
        "expected 192.168.10.2/24 in {addrs:?}"
    );
}

/// Verifies reconcile is a no-op before initialization completes.
#[tokio::test]
async fn reconcile_before_initialization_is_a_noop() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        Some(event_rx),
        config_static_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(
        ipv4_addrs(&mock, idx).is_empty(),
        "reconcile should do nothing before initialization"
    );
}

/// Verifies supervisor reconcile restores drifted static IPv6 state.
#[tokio::test]
async fn reconcile_reapplies_static_ipv6_after_drift() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        Some(event_rx),
        config_static_ipv6_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    handle.initialize_with_retry().await.expect("init failed");

    mock.remove_ipv6(idx, "2001:db8::2".parse().expect("valid ipv6"))
        .await
        .expect("remove failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

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

/// Verifies reconcile can safely drive the DHCP path.
#[tokio::test]
async fn reconcile_with_dhcp_config_completes() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_dhcp_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    let result = handle.reconcile().await;

    // ASSERT
    assert!(result.is_ok(), "DHCP reconcile should complete");
}

/// Verifies reconcile skips bridge creation when the configured port is unresolved.
#[tokio::test]
async fn reconcile_skips_bridge_when_port_is_missing() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock.clone(),
        Some(event_rx),
        config_bridge_missing_port(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    handle.reconcile().await.expect("reconcile failed");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    assert!(!has_link(&mock, "br0"), "bridge should not be created");
}

/// Verifies reconcile tolerates a configured interface that is absent at runtime.
#[tokio::test]
async fn reconcile_named_missing_interface_is_non_fatal() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_static_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    handle.initialize_with_retry().await.expect("init failed");

    // ACT
    let result = handle.reconcile().await;

    // ASSERT
    assert!(
        result.is_ok(),
        "reconcile should log and continue on missing interface"
    );
}
