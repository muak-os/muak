//! Integration tests for DNS conf writing via the interface actor.

use std::sync::Arc;
use std::time::Duration;

use networkd::interface::dns::DnsState;

use super::{InterfaceActor, InterfaceCommand, InterfaceState};

fn make_config() -> Arc<config::NetworkConfig> {
    Arc::new(config::NetworkConfig {
        dns: vec!["8.8.8.8".to_string(), "2001:4860:4860::8888".to_string()],
        interfaces: vec![],
        ..Default::default()
    })
}

#[tokio::test]
async fn static_ipv4_writes_nameservers_to_resolv_conf() {
    // ARRANGE
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolv_conf = tmp.path().join("resolv.conf");
    let dns = DnsState::with_path(resolv_conf.clone());

    let mock = super::MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = super::make_snapshot("eth0", idx, [0xAA; 6]);
    let handle = InterfaceActor::spawn_with_dns(snapshot, mock.clone(), make_config(), dns);

    let addr = config::Cidr4 {
        address: std::net::Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");

    // ASSERT
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = handle.state_rx.clone();
        while rx.borrow().state != InterfaceState::Configured {
            rx.changed().await.expect("actor dropped");
        }
    })
    .await
    .expect("timed out waiting for Configured");

    let content = std::fs::read_to_string(&resolv_conf).expect("resolv.conf not written");
    assert!(
        content.contains("nameserver 8.8.8.8"),
        "missing v4 entry: {content}"
    );
}

#[tokio::test]
async fn static_ipv6_writes_nameservers_to_resolv_conf() {
    // ARRANGE
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolv_conf = tmp.path().join("resolv.conf");
    let dns = DnsState::with_path(resolv_conf.clone());

    let mock = super::MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);
    let snapshot = super::make_snapshot("eth1", idx, [0xBB; 6]);
    let handle = InterfaceActor::spawn_with_dns(snapshot, mock.clone(), make_config(), dns);

    let addr = config::Cidr6 {
        address: "fd00::2".parse().expect("valid addr"),
        prefix: 64,
    };

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");

    // ASSERT
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = handle.state_rx.clone();
        while rx.borrow().ipv6.is_none() {
            rx.changed().await.expect("actor dropped");
        }
    })
    .await
    .expect("timed out waiting for ipv6");

    let content = std::fs::read_to_string(&resolv_conf).expect("resolv.conf not written");
    assert!(
        content.contains("nameserver 2001:4860:4860::8888"),
        "missing v6 entry: {content}"
    );
}
