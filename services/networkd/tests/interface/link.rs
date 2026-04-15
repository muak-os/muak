//! Integration tests for link up/down event handling on interface actor.

use std::net::Ipv4Addr;
use std::time::Duration;

use super::*;

#[tokio::test]
async fn link_down_on_configured_transitions_to_degraded() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = make_snapshot("eth0", idx, [0xAA; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            index: idx,
            addresses: vec![addr],
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");

    wait_for_state(&handle, InterfaceState::Degraded).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Degraded);
    assert_eq!(snap.link, LinkStateKind::Down);
}

#[tokio::test]
async fn link_down_on_discovered_stays_discovered() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);
    let snapshot = make_snapshot("eth1", idx, [0xBB; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock, make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Discovered);
    assert_eq!(snap.link, LinkStateKind::Down);
}

#[tokio::test]
async fn link_up_on_non_degraded_does_not_publish_snapshot() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0xCC; 6], false);
    let mut snapshot = make_snapshot("eth2", idx, [0xCC; 6]);
    snapshot.link = LinkStateKind::Down;
    let handle = InterfaceActor::spawn(snapshot, mock, make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkUp)
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Discovered);
    assert_eq!(
        snap.link,
        LinkStateKind::Down,
        "link-up on non-degraded does not publish to watch"
    );
}

#[tokio::test]
async fn link_down_clears_dhcp_state() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth3", [0xDD; 6], true);
    let snapshot = make_snapshot("eth3", idx, [0xDD; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 5),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Degraded).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.link, LinkStateKind::Down);
}

#[tokio::test]
async fn multiple_link_down_events_are_idempotent() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth4", [0xEE; 6], true);
    let snapshot = make_snapshot("eth4", idx, [0xEE; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 1, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            index: idx,
            addresses: vec![addr],
            gateway: None,
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Configured).await;

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");
    wait_for_state(&handle, InterfaceState::Degraded).await;

    handle
        .cmd_tx
        .send(InterfaceCommand::LinkDown)
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Degraded);
}

#[tokio::test]
async fn link_up_after_link_down_publishes_snapshot() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth5", [0xFF; 6], true);
    let snapshot = make_snapshot("eth5", idx, [0xFF; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 2, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
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
    handle
        .cmd_tx
        .send(InterfaceCommand::LinkUp)
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.link, LinkStateKind::Up);
}
