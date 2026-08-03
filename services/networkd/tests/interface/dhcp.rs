//! Integration tests for DHCP lease life cycle in interface actor.

use core::net::Ipv4Addr;
use core::time::Duration;

use networkd::interface::commands::ApplyMode;
use tokio::time::sleep;

use super::*;

#[tokio::test]
async fn configure_dhcp_transitions_to_configuring() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = make_snapshot(Name::new("eth0").expect("valid name"), idx, [0xAA; 6]);
    let handle = Actor::spawn_with(snapshot, mock, make_config(), MockDhcpConnector);

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureDhcp {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, Lifecycle::Configuring).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, Lifecycle::Configuring);
}

#[tokio::test]
async fn link_down_on_dhcp_configured_transitions_to_degraded() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);
    let snapshot = make_snapshot(Name::new("eth1").expect("valid name"), idx, [0xBB; 6]);
    let handle = Actor::spawn(snapshot, mock.clone(), make_config());

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(Command::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, Lifecycle::Configured).await;

    // ACT
    handle
        .cmd_tx
        .send(Command::LinkDown)
        .await
        .expect("send failed");
    wait_for_state(&handle, Lifecycle::Degraded).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, Lifecycle::Degraded);
    assert_eq!(snap.link, State::Down);
}

#[tokio::test]
async fn configure_dhcp_leaves_dhcp_state_none_before_dora() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0xCC; 6], true);
    let snapshot = make_snapshot(Name::new("eth2").expect("valid name"), idx, [0xCC; 6]);
    let handle = Actor::spawn_with(snapshot, mock, make_config(), MockDhcpConnector);

    // ACT
    handle
        .cmd_tx
        .send(Command::ConfigureDhcp {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, Lifecycle::Configuring).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert!(
        snap.dhcp_state.is_none(),
        "dhcp_state should be None before DORA completes"
    );
}

#[tokio::test]
async fn shutdown_while_dhcp_configuring_stops_actor() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth3", [0xDD; 6], true);
    let snapshot = make_snapshot(Name::new("eth3").expect("valid name"), idx, [0xDD; 6]);
    let handle = Actor::spawn_with(snapshot, mock, make_config(), MockDhcpConnector);

    handle
        .cmd_tx
        .send(Command::ConfigureDhcp {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");
    wait_for_state(&handle, Lifecycle::Configuring).await;

    // ACT
    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send failed");
    sleep(Duration::from_millis(50)).await;

    // ASSERT
    let result = handle.cmd_tx.send(Command::LinkUp).await;
    assert!(result.is_err(), "channel should be closed after shutdown");
}
