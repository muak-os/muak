//! Integration tests for interface actor reconciliation commands.

use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;

use netlib::address;
use netlib::address::IpConfig;
use networkd::dhcp::Lease;
use networkd::dhcp::State as DhcpState;
use networkd::interface::commands::ApplyMode;
use tokio::time::sleep;
use tokio::time::timeout;

use super::*;

/// Waits until a mock interface regains the expected IPv4 address.
async fn wait_for_ipv4_addr(mock: &MockNetlinkOps, index: u32, address: Ipv4Addr, prefix: u8) {
    // ARRANGE
    let timeout_duration = Duration::from_secs(5);

    // ACT
    let result = timeout(timeout_duration, async {
        while !mock.ipv4_addrs(index).contains(&(address, prefix)) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    // ASSERT
    assert!(result.is_ok(), "timed out waiting for IPv4 address");
}

/// Waits until a mock interface regains the expected IPv6 address.
async fn wait_for_ipv6_addr(mock: &MockNetlinkOps, index: u32, address: Ipv6Addr, prefix: u8) {
    // ARRANGE
    let timeout_duration = Duration::from_secs(5);

    // ACT
    let result = timeout(timeout_duration, async {
        while !mock.ipv6_addrs(index).contains(&(address, prefix)) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    // ASSERT
    assert!(result.is_ok(), "timed out waiting for IPv6 address");
}

/// Verifies static IPv4 reconciliation re-applies drifted kernel state.
#[tokio::test]
async fn reconcile_static_ipv4_reapplies_kernel_state() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = make_snapshot(Name::new("eth0").expect("valid name"), idx, [0xAA; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let address = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    let gateway = Ipv4Addr::new(10, 0, 0, 1);

    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, Lifecycle::Configured).await;

    address::Ops::remove_ipv4(&mock, idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv4 {
            mode: ApplyMode::Reconcile,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");

    wait_for_ipv4_addr(&mock, idx, address.address, address.prefix).await;

    // ASSERT
    let addrs = mock.ipv4_addrs(idx);
    assert!(
        addrs.contains(&(Ipv4Addr::new(10, 0, 0, 2), 24)),
        "expected 10.0.0.2/24 in {addrs:?}"
    );
    assert!(
        mock.has_default_route_v4(gateway),
        "expected default route via {gateway}"
    );
}

/// Verifies DHCP reconciliation re-applies a cached lease.
#[tokio::test]
async fn reconcile_dhcp_with_cached_lease_reapplies_address() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);

    let address = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 10),
        prefix: 24,
    };
    let gateway = Ipv4Addr::new(10, 0, 0, 1);
    let lease = Lease {
        obtained_at: std::time::SystemTime::now(),
        lease_time: core::time::Duration::from_hours(1),
        renewal_time: core::time::Duration::from_mins(30),
        rebind_time: core::time::Duration::from_secs(3150),
        server_ip: gateway,
        assigned_ip: address.address,
        prefix_len: address.prefix,
        gateway: Some(gateway),
        dns_servers: vec![Ipv4Addr::new(1, 1, 1, 1)],
    };
    let snapshot = Snapshot {
        name: Name::new("eth1").expect("valid name"),
        state: Lifecycle::Configured,
        index: idx,
        mac: [0xBB; 6],
        link: State::Up,
        ip: Some(IpConfig {
            address: lease.assigned_ip,
            prefix_len: lease.prefix_len,
            gateway: lease.gateway,
            dns: lease.dns_servers.clone(),
        }),
        lease: Some(lease),
        dhcp_state: Some(DhcpState::Bound),
        ipv6: None,
        l3_owner: Name::new("eth1").expect("valid name"),
    };

    let reconciler = Actor::spawn(snapshot, mock.clone(), make_config());
    address::Ops::remove_ipv4(&mock, idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    reconciler
        .cmd_tx
        .send(Command::ConfigureDhcp {
            mode: ApplyMode::Reconcile,
        })
        .await
        .expect("send failed");

    wait_for_ipv4_addr(&mock, idx, address.address, address.prefix).await;

    // ASSERT
    let addrs = mock.ipv4_addrs(idx);
    assert!(
        addrs.contains(&(address.address, address.prefix)),
        "expected {:?} in {addrs:?}",
        (address.address, address.prefix)
    );
    assert!(
        mock.has_default_route_v4(gateway),
        "expected default route via {gateway}"
    );
}

/// Verifies static IPv6 reconciliation re-applies drifted kernel state.
#[tokio::test]
async fn reconcile_static_ipv6_reapplies_kernel_state() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0xCC; 6], true);
    let snapshot = make_snapshot(Name::new("eth2").expect("valid name"), idx, [0xCC; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let address = config::Cidr6 {
        address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
        prefix: 64,
    };
    let gateway = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, Lifecycle::Configured).await;

    address::Ops::remove_ipv6(&mock, idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv6 {
            mode: ApplyMode::Reconcile,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");

    wait_for_ipv6_addr(&mock, idx, address.address, address.prefix).await;

    // ASSERT
    let addrs = mock.ipv6_addrs(idx);
    assert!(
        addrs.contains(&(address.address, address.prefix)),
        "expected {:?} in {addrs:?}",
        (address.address, address.prefix)
    );
    assert!(
        mock.has_default_route_v6(gateway),
        "expected default route via {gateway}"
    );
}
