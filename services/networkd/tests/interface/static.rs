//! Integration tests for static IP configuration edge cases on interface actor.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use networkd::interface::ApplyMode;

use super::*;

#[tokio::test]
async fn static_ipv4_multiple_addresses() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0x01; 6], true);
    let snapshot = make_snapshot("eth0", idx, [0x01; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addrs = vec![
        config::Cidr4 {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix: 24,
        },
        config::Cidr4 {
            address: Ipv4Addr::new(10, 0, 0, 3),
            prefix: 24,
        },
    ];

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: addrs,
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ASSERT
    let ip_addrs = mock.ipv4_addrs(idx);
    assert!(
        ip_addrs.contains(&(Ipv4Addr::new(10, 0, 0, 2), 24)),
        "first address missing"
    );
    assert!(
        ip_addrs.contains(&(Ipv4Addr::new(10, 0, 0, 3), 24)),
        "second address missing"
    );
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(
        snap.ip.as_ref().expect("ip").address,
        Ipv4Addr::new(10, 0, 0, 2)
    );
    assert_eq!(snap.ip.as_ref().expect("ip").prefix_len, 24);
}

#[tokio::test]
async fn static_ipv6_without_gateway_skips_route() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0x02; 6], true);
    let snapshot = make_snapshot("eth1", idx, [0x02; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr6 {
        address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
        prefix: 64,
    };

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");
    wait_for_ipv6(&handle).await;

    // ASSERT
    let addrs = mock.ipv6_addrs(idx);
    assert!(
        addrs.contains(&(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2), 64)),
        "expected address in {addrs:?}"
    );
    assert!(
        !mock.has_default_route_v6(Ipv6Addr::UNSPECIFIED),
        "no v6 route should exist"
    );
}

#[tokio::test]
async fn static_ipv6_multiple_addresses() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0x03; 6], true);
    let snapshot = make_snapshot("eth2", idx, [0x03; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addrs = vec![
        config::Cidr6 {
            address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
            prefix: 64,
        },
        config::Cidr6 {
            address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
            prefix: 64,
        },
    ];
    let gateway = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: addrs,
            gateway: Some(gateway),
        })
        .await
        .expect("send failed");
    wait_for_ipv6(&handle).await;

    // ASSERT
    let v6_addrs = mock.ipv6_addrs(idx);
    assert!(
        v6_addrs.contains(&(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 64)),
        "first v6 address missing"
    );
    assert!(
        v6_addrs.contains(&(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2), 64)),
        "second v6 address missing"
    );
    assert!(mock.has_default_route_v6(gateway), "v6 route missing");
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(
        snap.ipv6.as_ref().expect("ipv6").address,
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)
    );
}

#[tokio::test]
async fn static_ipv4_then_shutdown_cleans_up() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth3", [0x04; 6], true);
    let snapshot = make_snapshot("eth3", idx, [0x04; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(172, 16, 0, 10),
        prefix: 16,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: Some(Ipv4Addr::new(172, 16, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::Shutdown)
        .await
        .expect("send failed");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ASSERT
    let result = handle.cmd_tx.send(InterfaceCommand::LinkUp).await;
    assert!(result.is_err(), "channel should be closed after shutdown");
}

#[tokio::test]
async fn static_ipv4_and_ipv6_on_same_interface() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth4", [0x05; 6], true);
    let snapshot = make_snapshot("eth4", idx, [0x05; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let v4_addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 10, 0, 2),
        prefix: 24,
    };
    let v6_addr = config::Cidr6 {
        address: Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2),
        prefix: 64,
    };

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![v4_addr],
            gateway: Some(Ipv4Addr::new(10, 10, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            mode: ApplyMode::Reconcile,
            index: idx,
            addresses: vec![v6_addr],
            gateway: Some(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_ipv6(&handle).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert!(snap.ip.is_some(), "v4 config should exist");
    assert!(snap.ipv6.is_some(), "v6 config should exist");
    assert_eq!(
        snap.ip.as_ref().expect("ip").address,
        Ipv4Addr::new(10, 10, 0, 2)
    );
    assert_eq!(
        snap.ipv6.as_ref().expect("ipv6").address,
        Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2)
    );
}

#[tokio::test]
async fn static_ipv4_reaches_configured_from_discovered() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth5", [0x06; 6], true);
    let snapshot = make_snapshot("eth5", idx, [0x06; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 99),
        prefix: 24,
    };

    let initial_state = handle.state_rx.borrow().state.clone();
    assert_eq!(initial_state, InterfaceState::Discovered);

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, InterfaceState::Configured).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Configured);
    assert_eq!(
        snap.ip.as_ref().expect("ip").address,
        Ipv4Addr::new(10, 0, 0, 99)
    );
}

#[tokio::test]
async fn link_down_then_static_reconfigure_from_degraded() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth6", [0x07; 6], true);
    let snapshot = make_snapshot("eth6", idx, [0x07; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 3, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Degraded).await;

    // ACT
    let new_addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 3, 5),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![new_addr],
            gateway: Some(Ipv4Addr::new(10, 0, 3, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Configured);
    assert_eq!(
        snap.ip.as_ref().expect("ip").address,
        Ipv4Addr::new(10, 0, 3, 5)
    );
    assert!(mock.has_default_route_v4(Ipv4Addr::new(10, 0, 3, 1)));
}

#[tokio::test]
async fn static_ipv4_empty_addresses_stays_configuring() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth7", [0x08; 6], true);
    let snapshot = make_snapshot("eth7", idx, [0x08; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![],
            gateway: None,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(
        snap.state,
        InterfaceState::Configuring,
        "empty addresses should leave actor in Configuring (error logged)"
    );
    assert!(snap.ip.is_none(), "no IP should be set");
}

#[tokio::test]
async fn static_ipv6_empty_addresses_does_not_configure() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth8", [0x09; 6], true);
    let snapshot = make_snapshot("eth8", idx, [0x09; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv6 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![],
            gateway: None,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert!(snap.ipv6.is_none(), "no IPv6 should be set");
}
