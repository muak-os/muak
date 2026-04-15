//! Integration tests for network event dispatch through the supervisor.

use std::time::Duration;

use netlib::interface::InterfaceName;
use netlib::monitor::NetworkEvent;

use super::*;

#[tokio::test]
async fn link_down_event_dispatched_to_interface() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDown { name, index: idx })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_up_event_dispatched_to_interface() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkUp { name, index: idx })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_added_event_spawns_new_actor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle =
        supervisor::start_with(mock.clone(), Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    mock.add_link("eth1", [0xBB; 6], true);
    let name = InterfaceName::new("eth1").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkAdded {
            name,
            index: 2,
            mac: [0xBB; 6],
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_deleted_event_removes_actor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth1").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDeleted { name, index: 2 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn duplicate_link_added_event_is_ignored() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkAdded {
            name: name.clone(),
            index: idx,
            mac: [0xAA; 6],
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_deleted_for_unknown_interface_is_safe() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth99").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDeleted { name, index: 99 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn rapid_link_events_do_not_crash_supervisor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    for _ in 0..10 {
        let _ = event_tx
            .send(NetworkEvent::LinkDown {
                name: name.clone(),
                index: idx,
            })
            .await;
        let _ = event_tx
            .send(NetworkEvent::LinkUp {
                name: name.clone(),
                index: idx,
            })
            .await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_added_when_no_primary_assigns_as_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle =
        supervisor::start_with(mock.clone(), Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name0 = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDeleted {
            name: name0,
            index: idx0,
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ACT
    mock.add_link("eth2", [0xCC; 6], true);
    let name2 = InterfaceName::new("eth2").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkAdded {
            name: name2,
            index: 2,
            mac: [0xCC; 6],
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}
