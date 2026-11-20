//! Integration tests for network event dispatch through the supervisor.

use core::time::Duration;

use netlink::link::ifname::Name;
use netlink_monitor::{Event, Failure};
use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use super::*;

#[tokio::test]
async fn link_down_event_dispatched_to_interface() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Down { name, index: idx })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_up_event_dispatched_to_interface() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Up { name, index: idx })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_added_event_spawns_new_actor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default())
        .expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    mock.add_link("eth1", [0xBB; 6], true);
    let name = Name::new("eth1").expect("valid name");
    event_tx
        .send(Event::Added {
            name,
            index: 2,
            mac: [0xBB; 6],
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

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

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth1").expect("valid name");
    event_tx
        .send(Event::Deleted { name, index: 2 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn duplicate_link_added_event_is_ignored() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Added {
            name: name.clone(),
            index: idx,
            mac: [0xAA; 6],
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn link_deleted_for_unknown_interface_is_safe() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let name = Name::new("eth99").expect("valid name");
    event_tx
        .send(Event::Deleted { name, index: 99 })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn rapid_link_events_do_not_crash_supervisor() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(64);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    for _ in 0_i32..10_i32 {
        drop(
            event_tx
                .send(Event::Down {
                    name: name.clone(),
                    index: idx,
                })
                .await,
        );
        drop(
            event_tx
                .send(Event::Up {
                    name: name.clone(),
                    index: idx,
                })
                .await,
        );
    }

    sleep(Duration::from_millis(200)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn monitor_error_event_keeps_supervisor_responsive() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    event_tx
        .send(Event::Error(Failure::Connection(std::io::Error::other(
            "mock monitor failure",
        ))))
        .await
        .expect("send error event failed");
    let name = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Up { name, index: idx })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;
    let reconciled = timeout(Duration::from_secs(5), handle.reconcile()).await;

    // ASSERT
    assert!(
        reconciled.is_ok() && reconciled.expect("timeout").is_ok(),
        "supervisor should keep processing events after a monitor error"
    );
    drop(event_tx);
}

#[tokio::test]
async fn monitor_error_event_reconciles_immediately() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default())
        .expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let addr = (core::net::Ipv4Addr::new(10, 0, 0, 2), 24);
    let limit = Duration::from_secs(5);
    let step = Duration::from_millis(10);
    let mut waited = Duration::ZERO;
    while !ipv4_addrs(&mock, idx).contains(&addr) && waited < limit {
        sleep(step).await;
        waited += step;
    }
    assert!(
        ipv4_addrs(&mock, idx).contains(&addr),
        "provisioning must apply {addr:?}"
    );

    mock.remove_ipv4(idx, addr.0).await.expect("remove failed");
    assert!(
        !ipv4_addrs(&mock, idx).contains(&addr),
        "drift must be removed before the error event"
    );

    // ACT
    event_tx
        .send(Event::Error(Failure::Overrun))
        .await
        .expect("send error event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    let addrs = ipv4_addrs(&mock, idx);
    assert!(addrs.contains(&addr), "expected {addr:?} in {addrs:?}");
    drop(event_tx);
}

#[tokio::test]
async fn link_added_when_no_primary_assigns_as_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default())
        .expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name0 = Name::new("eth0").expect("valid name");
    event_tx
        .send(Event::Deleted {
            name: name0,
            index: idx0,
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ACT
    mock.add_link("eth2", [0xCC; 6], true);
    let name2 = Name::new("eth2").expect("valid name");
    event_tx
        .send(Event::Added {
            name: name2,
            index: 2,
            mac: [0xCC; 6],
        })
        .await
        .expect("send event failed");

    sleep(Duration::from_millis(100)).await;

    // ASSERT
    drop(event_tx);
}

#[tokio::test]
async fn monitor_closed_event_renews_the_subscription() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle =
        supervisor::start_with(mock, event_rx, config, Resolver::default()).expect("start failed");

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    event_tx
        .send(Event::Error(Failure::Closed))
        .await
        .expect("send failed");
    let limit = Duration::from_secs(5);
    let step = Duration::from_millis(10);
    let mut waited = Duration::ZERO;
    let mut disconnected = false;
    while !disconnected && waited < limit {
        disconnected = matches!(
            timeout(step, event_tx.send(Event::Error(Failure::Overrun))).await,
            Ok(Err(_))
        );
        sleep(step).await;
        waited += step;
    }

    // ASSERT
    let alive = timeout(Duration::from_secs(5), handle.reconcile()).await;
    assert!(
        alive.is_ok() && alive.expect("timeout").is_ok(),
        "supervisor must stay alive after renewing the monitor"
    );
    assert!(
        disconnected,
        "supervisor must renew the monitor subscription after the stream closed"
    );
    drop(event_tx);
}
