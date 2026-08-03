//! Integration tests for interface actor behavior.

use core::net::Ipv4Addr;

use networkd::interface::commands::ApplyMode;
use tokio::time::sleep;

use super::*;

#[tokio::test]
async fn static_ipv4_configures_address_and_gateway() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01], true);
    let snapshot = make_snapshot(
        Name::new("eth0").expect("valid name"),
        idx,
        [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01],
    );
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    let gateway = Ipv4Addr::new(10, 0, 0, 1);

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, Lifecycle::Configured).await;

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

#[tokio::test]
async fn static_ipv4_without_gateway_skips_route() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0x00, 0x11, 0x22, 0x33, 0x44, 0x55], true);
    let snapshot = make_snapshot(
        Name::new("eth1").expect("valid name"),
        idx,
        [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
    );
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(192, 168, 1, 10),
        prefix: 24,
    };

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, Lifecycle::Configured).await;

    // ASSERT
    let addrs = mock.ipv4_addrs(idx);
    assert!(
        addrs.contains(&(Ipv4Addr::new(192, 168, 1, 10), 24)),
        "expected 192.168.1.10/24 in {addrs:?}"
    );
    assert!(
        !mock.has_default_route_v4(Ipv4Addr::UNSPECIFIED),
        "no default route should exist"
    );
}

#[tokio::test]
async fn static_ipv6_configures_address_and_gateway() {
    use core::net::Ipv6Addr;

    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0xDE, 0xAD, 0x00, 0x00, 0x00, 0x01], true);
    let snapshot = make_snapshot(
        Name::new("eth2").expect("valid name"),
        idx,
        [0xDE, 0xAD, 0x00, 0x00, 0x00, 0x01],
    );
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr6 {
        address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
        prefix: 64,
    };
    let gateway = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");

    wait_for_ipv6(&handle).await;

    // ASSERT
    let addrs = mock.ipv6_addrs(idx);
    assert!(
        addrs.contains(&(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2), 64)),
        "expected 2001:db8::2/64 in {addrs:?}"
    );
    assert!(
        mock.has_default_route_v6(gateway),
        "expected default v6 route via {gateway}"
    );
}

#[tokio::test]
async fn shutdown_command_stops_actor() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth3", [0x00; 6], true);
    let snapshot = make_snapshot(Name::new("eth3").expect("valid name"), idx, [0x00; 6]);
    let handle = Actor::spawn(snapshot, mock, make_config());

    // ACT
    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send failed");

    sleep(core::time::Duration::from_millis(50)).await;

    // ASSERT
    let result = handle.cmd_tx.send(Command::LinkUp).await;
    assert!(result.is_err(), "channel should be closed after shutdown");
}
