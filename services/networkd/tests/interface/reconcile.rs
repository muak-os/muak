//! Integration tests for interface actor reconciliation commands.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use networkd::interface::ApplyMode;

use super::*;

/// Waits until a mock interface regains the expected IPv4 address.
async fn wait_for_ipv4_addr(mock: &MockNetlinkOps, index: u32, address: Ipv4Addr, prefix: u8) {
    // ARRANGE
    let timeout = Duration::from_secs(5);

    // ACT
    let result = tokio::time::timeout(timeout, async {
        loop {
            if mock.ipv4_addrs(index).contains(&(address, prefix)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    // ASSERT
    assert!(result.is_ok(), "timed out waiting for IPv4 address");
}

/// Waits until a mock interface regains the expected IPv6 address.
async fn wait_for_ipv6_addr(mock: &MockNetlinkOps, index: u32, address: Ipv6Addr, prefix: u8) {
    // ARRANGE
    let timeout = Duration::from_secs(5);

    // ACT
    let result = tokio::time::timeout(timeout, async {
        loop {
            if mock.ipv6_addrs(index).contains(&(address, prefix)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
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
    let snapshot = make_snapshot("eth0", idx, [0xAA; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let address = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    let gateway = Ipv4Addr::new(10, 0, 0, 1);

    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    mock.remove_ipv4(idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
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
    let lease = networkd::dhcp::DhcpLease {
        obtained_at: std::time::SystemTime::now(),
        lease_time: std::time::Duration::from_secs(3600),
        renewal_time: std::time::Duration::from_secs(1800),
        rebind_time: std::time::Duration::from_secs(3150),
        server_ip: gateway,
        assigned_ip: address.address,
        prefix_len: address.prefix,
        gateway: Some(gateway),
        dns_servers: vec![Ipv4Addr::new(1, 1, 1, 1)],
    };
    let snapshot = InterfaceSnapshot {
        name: InterfaceName::new("eth1").expect("valid name"),
        state: InterfaceState::Configured,
        index: idx,
        mac: [0xBB; 6],
        link: LinkStateKind::Up,
        ip: Some(netlib::address::IpConfig {
            address: lease.assigned_ip,
            prefix_len: lease.prefix_len,
            gateway: lease.gateway,
            dns: lease.dns_servers.clone(),
        }),
        lease: Some(lease),
        dhcp_state: Some(networkd::dhcp::DhcpState::Bound),
        ipv6: None,
        l3_owner: InterfaceName::new("eth1").expect("valid name"),
    };

    let reconciler = InterfaceActor::spawn(snapshot, mock.clone(), make_config());
    mock.remove_ipv4(idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    reconciler
        .cmd_tx
        .send(InterfaceCommand::ConfigureDhcp {
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
    let snapshot = make_snapshot("eth2", idx, [0xCC; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let address = config::Cidr6 {
        address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
        prefix: 64,
    };
    let gateway = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![address],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    mock.remove_ipv6(idx, address.address)
        .await
        .expect("remove failed");

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
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
