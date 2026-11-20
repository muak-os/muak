//! Integration tests for the supervisor failover logic.

use core::time::Duration;

use netlink::link::ifname::Name;
use netlink_monitor::Event;
use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use super::*;

#[tokio::test]
async fn primary_link_down_triggers_failover_handling() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Down { name, index: idx0 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(200)).await;

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

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    event_tx
        .send(Event::Up { name, index: idx0 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

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

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let backup_name = Name::new("eth1").expect("valid name");

    // ACT
    event_tx
        .send(Event::Up {
            name: backup_name,
            index: idx1,
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

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

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Deleted { name, index: idx0 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn single_interface_deleted_leaves_no_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Deleted { name, index: idx0 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(200)).await;

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

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name0 = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Deleted {
            name: name0,
            index: idx0,
        })
        .await
        .expect("send");

    sleep(Duration::from_millis(100)).await;

    let name1 = Name::new("eth1").expect("valid name");
    event_tx
        .send(Event::Deleted {
            name: name1,
            index: idx1,
        })
        .await
        .expect("send");

    sleep(Duration::from_millis(200)).await;

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

    let (event_tx, event_rx) = mpsc::channel(64);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    for _ in 0_i32..5_i32 {
        drop(
            event_tx
                .send(Event::Down {
                    name: name.clone(),
                    index: idx0,
                })
                .await,
        );
        drop(
            event_tx
                .send(Event::Up {
                    name: name.clone(),
                    index: idx0,
                })
                .await,
        );
    }

    sleep(Duration::from_millis(300)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn primary_link_up_when_ready_keeps_supervisor_alive() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Up { name, index: idx0 })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    timeout(Duration::from_secs(5), handle.reconcile())
        .await
        .expect("reconcile timed out")
        .expect("the supervisor must stay responsive");
    drop(event_tx);
}

#[tokio::test]
async fn primary_down_then_backup_up_keeps_supervisor_alive() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    // ACT
    event_tx
        .send(Event::Down {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(100)).await;

    event_tx
        .send(Event::Up {
            name: backup_name,
            index: idx1,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    timeout(Duration::from_secs(5), handle.reconcile())
        .await
        .expect("reconcile timed out")
        .expect("the supervisor must stay responsive");
    drop(event_tx);
}

#[tokio::test]
async fn double_link_down_on_primary_keeps_supervisor_alive() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(100)).await;

    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    timeout(Duration::from_secs(5), handle.reconcile())
        .await
        .expect("reconcile timed out")
        .expect("the supervisor must stay responsive");
    drop(event_tx);
}

#[tokio::test]
async fn delete_last_interface_when_degraded_keeps_supervisor_alive() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(100)).await;

    event_tx
        .send(Event::Deleted {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    timeout(Duration::from_secs(5), handle.reconcile())
        .await
        .expect("reconcile timed out")
        .expect("the supervisor must stay responsive");
    drop(event_tx);
}
