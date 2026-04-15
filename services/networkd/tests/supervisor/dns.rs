//! Integration tests verifying the supervisor writes DNS on behalf of the primary interface.

use std::sync::Arc;
use std::time::Duration;

use networkd::dns::DnsState;
use networkd::supervisor;

use super::MockNetlinkOps;

fn config_with_ipv4_dns() -> Arc<config::NetworkConfig> {
    Arc::new(config::NetworkConfig {
        dns: vec!["8.8.8.8".to_string()],
        interfaces: vec![config::InterfaceConfig {
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
        }],
        ..Default::default()
    })
}

fn config_with_ipv6_dns() -> Arc<config::NetworkConfig> {
    Arc::new(config::NetworkConfig {
        dns: vec!["2001:4860:4860::8888".to_string()],
        interfaces: vec![config::InterfaceConfig {
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
            ipv6: Some(config::Ipv6InterfaceConfig {
                addresses: vec![config::Cidr6 {
                    address: "fd00::2".parse().expect("valid addr"),
                    prefix: 64,
                }],
                gateway: None,
                autoconf: false,
            }),
            bridge: None,
        }],
        ..Default::default()
    })
}

#[tokio::test]
async fn static_ipv4_supervisor_writes_nameservers_to_resolv_conf() {
    // ARRANGE
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolv_conf = tmp.path().join("resolv.conf");
    let dns = DnsState::with_path(resolv_conf.clone());

    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config_with_ipv4_dns(), dns)
        .expect("start failed");

    // ACT
    tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry())
        .await
        .expect("timed out")
        .expect("init failed");

    // ASSERT
    let content = std::fs::read_to_string(&resolv_conf).expect("resolv.conf not written");
    assert!(
        content.contains("nameserver 8.8.8.8"),
        "missing v4 entry: {content}"
    );
}

#[tokio::test]
async fn static_ipv6_supervisor_writes_nameservers_to_resolv_conf() {
    // ARRANGE
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolv_conf = tmp.path().join("resolv.conf");
    let dns = DnsState::with_path(resolv_conf.clone());

    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config_with_ipv6_dns(), dns)
        .expect("start failed");

    // ACT
    tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry())
        .await
        .expect("timed out")
        .expect("init failed");

    // ASSERT
    let content = std::fs::read_to_string(&resolv_conf).expect("resolv.conf not written");
    assert!(
        content.contains("nameserver 2001:4860:4860::8888"),
        "missing v6 entry: {content}"
    );
}
