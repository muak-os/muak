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
    let addrs = mock.ipv4_addrs(idx);
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
        mock.ipv4_addrs(idx).is_empty(),
        "reconcile should do nothing before initialization"
    );
}
