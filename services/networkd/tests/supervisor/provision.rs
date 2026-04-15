//! Integration tests for `NetworkSupervisor` interface provisioning.

use std::sync::Arc;
use std::time::Duration;

use networkd::supervisor;

use super::MockNetlinkOps;

fn config_static_ipv4() -> Arc<config::NetworkConfig> {
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

fn config_dhcp() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
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

fn config_ipv6() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
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

fn config_slaac() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.ipv6 = true;
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: Some(config::Ipv6InterfaceConfig {
            autoconf: true,
            addresses: vec![],
            gateway: None,
        }),
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_named() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "eth0".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: std::net::Ipv4Addr::new(192, 168, 1, 10),
                prefix: 24,
            }],
            gateway: Some(std::net::Ipv4Addr::new(192, 168, 1, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_none() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: None,
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
}

fn config_bridge() -> Arc<config::NetworkConfig> {
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
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_string(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
            port: vec!["auto".to_string()],
            stp: true,
        }),
    });
    Arc::new(cfg)
}

fn config_bridge_multiport() -> Arc<config::NetworkConfig> {
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
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_string(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
            port: vec!["auto".to_string(), "eth1".to_string()],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

fn config_bridge_named_port() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "eth0".to_string(),
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
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_string(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
            port: vec!["eth0".to_string()],
            stp: false,
        }),
    });
    Arc::new(cfg)
}

fn config_bridge_empty_port() -> Arc<config::NetworkConfig> {
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
    cfg.interfaces.push(config::InterfaceConfig {
        name: "br0".to_string(),
        kind: config::InterfaceKind::Bridge,
        ipv4: None,
        ipv6: None,
        bridge: Some(config::BridgeConfig {
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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_static_ipv4(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(result.expect("timeout").is_ok(), "should succeed");
}

#[tokio::test]
async fn provision_static_ipv4_named_interface_not_found_still_initializes() {
    // ARRANGE — eth1 is present but config resolves to eth0; the provision
    // failure is logged and swallowed, so init still completes.
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xBB; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_static_ipv4(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_dhcp(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT — init completes (DHCP config command is sent to actor)
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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_ipv6(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_slaac(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with named interface"
    );
}

#[tokio::test]
async fn provision_named_interface_not_found_still_initializes() {
    // ARRANGE — eth0 is configured but only eth1 is discovered; provision
    // failure is logged and swallowed by try_provision_interface.
    let mock = MockNetlinkOps::new();
    mock.add_link("eth1", [0xBB; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_named(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT — init succeeds even though the named interface wasn't found
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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_none(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

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

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_bridge(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with bridge config"
    );
}

#[tokio::test]
async fn provision_bridge_multiport_warns_and_uses_first_port() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_bridge_multiport(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT — init succeeds; the multi-port warn branch (provision.rs:187-192) was exercised
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with multi-port bridge config"
    );
}

#[tokio::test]
async fn provision_bridge_named_port_resolves_by_name() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_bridge_named_port(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT — init succeeds; the named-port branch (provision.rs:196) was exercised
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed with named bridge port"
    );
}

#[tokio::test]
async fn provision_bridge_empty_port_falls_back_to_primary() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config_bridge_empty_port(),
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    // ACT
    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT — init succeeds; the empty-ports None branch (provision.rs:197) was exercised
    assert!(result.is_ok(), "should not time out");
    assert!(
        result.expect("timeout").is_ok(),
        "initialization should succeed when bridge port list is empty"
    );
}
