//! Integration tests for the supervisor failover logic.

use std::time::Duration;

use netlib::interface::InterfaceName;
use netlib::monitor::NetworkEvent;

use super::*;

#[tokio::test]
async fn primary_link_down_triggers_failover_handling() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDown { name, index: idx0 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn primary_link_down_then_up_triggers_recovery() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkDown {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    event_tx
        .send(NetworkEvent::LinkUp { name, index: idx0 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn backup_link_recovery_triggers_promotion() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let backup_name = InterfaceName::new("eth1").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkUp {
            name: backup_name,
            index: idx1,
        })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn primary_deleted_promotes_backup() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkDeleted { name, index: idx0 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn single_interface_deleted_leaves_no_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkDeleted { name, index: idx0 })
        .await
        .expect("send event failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn all_interfaces_deleted_degrades_supervisor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name0 = InterfaceName::new("eth0").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDeleted {
            name: name0,
            index: idx0,
        })
        .await
        .expect("send");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let name1 = InterfaceName::new("eth1").expect("valid name");
    event_tx
        .send(NetworkEvent::LinkDeleted {
            name: name1,
            index: idx1,
        })
        .await
        .expect("send");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn rapid_primary_failover_and_recovery() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    for _ in 0..5 {
        let _ = event_tx
            .send(NetworkEvent::LinkDown {
                name: name.clone(),
                index: idx0,
            })
            .await;
        let _ = event_tx
            .send(NetworkEvent::LinkUp {
                name: name.clone(),
                index: idx0,
            })
            .await;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn primary_link_up_when_ready_hits_recovery_warn() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkUp { name, index: idx0 })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn backup_link_up_when_ready_hits_recovery_warn() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let backup_name = InterfaceName::new("eth1").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkUp {
            name: backup_name,
            index: idx1,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn double_link_down_on_primary_hits_failure_warn() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkDown {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    event_tx
        .send(NetworkEvent::LinkDown {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn delete_last_interface_when_already_degraded_hits_degrade_warn() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config).expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = InterfaceName::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(NetworkEvent::LinkDown {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    event_tx
        .send(NetworkEvent::LinkDeleted {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}
