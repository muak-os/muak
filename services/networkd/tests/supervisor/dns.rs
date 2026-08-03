//! Integration tests verifying the supervisor writes DNS on behalf of the primary interface.

use alloc::sync::Arc;
use core::time::Duration;

use networkd::dns::Resolver;
use networkd::supervisor;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::MockNetlinkOps;

fn config_with_ipv4_dns() -> Arc<config::NetworkConfig> {
    let sys = config::parse_from_str(
        r#"
[host]
name = "test"
port = 50051
ntp = "pool.ntp.org"

[network]
dns = ["8.8.8.8"]

[[network.interfaces]]
name = "auto"
type = "ethernet"
ipv4.dhcp = false
ipv4.addresses = ["10.0.0.2/24"]
ipv4.gateway = "10.0.0.1"
"#,
    )
    .expect("valid config");
    Arc::new(sys.network)
}

fn config_with_ipv6_dns() -> Arc<config::NetworkConfig> {
    let sys = config::parse_from_str(
        r#"
[host]
name = "test"
port = 50051
ntp = "pool.ntp.org"

[network]
dns = ["2001:4860:4860::8888"]

[[network.interfaces]]
name = "auto"
type = "ethernet"
ipv4.dhcp = false
ipv4.addresses = ["10.0.0.2/24"]
ipv4.gateway = "10.0.0.1"
ipv6.addresses = ["fd00::2/64"]
"#,
    )
    .expect("valid config");
    Arc::new(sys.network)
}

#[tokio::test]
async fn static_ipv4_supervisor_writes_nameservers_to_resolv_conf() {
    // ARRANGE
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolv_conf = tmp.path().join("resolv.conf");
    let dns = Resolver::with_path(resolv_conf.clone());

    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config_with_ipv4_dns(), dns)
        .expect("start failed");

    // ACT
    timeout(Duration::from_secs(5), handle.initialize_with_retry())
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
    let dns = Resolver::with_path(resolv_conf.clone());

    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config_with_ipv6_dns(), dns)
        .expect("start failed");

    // ACT
    timeout(Duration::from_secs(5), handle.initialize_with_retry())
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
