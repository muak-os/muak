//! Integration tests for SLAAC command handling on interface actor.

use std::time::Duration;

use networkd::interface::ApplyMode;

use super::*;

#[tokio::test]
async fn configure_slaac_does_not_crash_actor() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);
    let snapshot = make_snapshot("eth0", idx, [0xAA; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock, make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureSlaac {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    let result = handle.cmd_tx.send(InterfaceCommand::LinkUp).await;
    assert!(
        result.is_ok(),
        "actor should still be alive after SLAAC attempt"
    );
}

#[tokio::test]
async fn slaac_then_static_ipv4_both_work() {
    use std::net::Ipv4Addr;

    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth1", [0xBB; 6], true);
    let snapshot = make_snapshot("eth1", idx, [0xBB; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock.clone(), make_config());

    // ACT
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureSlaac {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let addr = config::Cidr4 {
        address: Ipv4Addr::new(10, 0, 0, 2),
        prefix: 24,
    };
    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureStaticIpv4 {
            mode: ApplyMode::Provision,
            index: idx,
            addresses: vec![addr],
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        })
        .await
        .expect("send failed");

    wait_for_state(&handle, InterfaceState::Configured).await;

    // ASSERT
    let snap = handle.state_rx.borrow().clone();
    assert_eq!(snap.state, InterfaceState::Configured);
    assert!(snap.ip.is_some());
}

#[tokio::test]
async fn slaac_then_shutdown_stops_cleanly() {
    // ARRANGE
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth2", [0xCC; 6], true);
    let snapshot = make_snapshot("eth2", idx, [0xCC; 6]);
    let handle = InterfaceActor::spawn(snapshot, mock, make_config());

    handle
        .cmd_tx
        .send(InterfaceCommand::ConfigureSlaac {
            mode: ApplyMode::Provision,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(50)).await;

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
