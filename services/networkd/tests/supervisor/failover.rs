//! Integration tests for the supervisor failover logic.

use core::time::Duration;

use netlink::link::ifname::Name;
use netlink::link::state::State;
use netlink_monitor::Event;
use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;
use crate::fixtures::status::{wait_for, wait_for_events};

#[tokio::test]
async fn primary_link_down_triggers_failover_handling() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    event_tx
        .send(Event::Down {
            name: primary_name.clone(),
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert_eq!(
        status.backups,
        vec![primary_name],
        "the failed primary must be the only backup after promotion"
    );
}

#[tokio::test]
async fn primary_link_down_then_up_triggers_recovery() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let before = handle
        .status()
        .await
        .expect("the supervisor must stay responsive")
        .processed_events;
    event_tx
        .send(Event::Down {
            name: primary_name.clone(),
            index: idx0,
        })
        .await
        .expect("send event failed");
    wait_for_events(&handle, before).await;

    event_tx
        .send(Event::Up {
            name: primary_name.clone(),
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Operational
            && status.primary.as_ref() == Some(&primary_name)
    })
    .await;
    assert_eq!(
        status.backups,
        vec![backup_name],
        "the recovered interface must reclaim the primary role"
    );

    drop(event_tx);
}

#[tokio::test]
async fn backup_recovery_keeps_the_configured_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let before = handle
        .status()
        .await
        .expect("the supervisor must stay responsive")
        .processed_events;
    event_tx
        .send(Event::Up {
            name: backup_name.clone(),
            index: idx1,
        })
        .await
        .expect("send event failed");
    let status = wait_for_events(&handle, before).await;

    // ASSERT
    assert_eq!(
        status.primary,
        Some(primary_name),
        "a configured primary must not be replaced"
    );
    assert_eq!(status.state, supervisor::NetworkState::Ready);
    assert_eq!(status.backups, vec![backup_name]);

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
    let backup_name = Name::new("eth1").expect("valid name");

    // ACT
    event_tx
        .send(Event::Deleted { name, index: idx0 })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert_eq!(status.state, supervisor::NetworkState::Ready);
    assert!(
        status.backups.is_empty(),
        "the promoted backup must leave no backups behind"
    );
    drop(event_tx);
}

#[tokio::test]
async fn primary_deletion_skips_a_carrier_less_backup() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    let idx1 = mock.add_link("eth1", [0xBB; 6], true);
    mock.add_link("eth2", [0xCC; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let down_backup = Name::new("eth1").expect("valid name");
    let healthy_backup = Name::new("eth2").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let before = handle
        .status()
        .await
        .expect("the supervisor must stay responsive")
        .processed_events;
    event_tx
        .send(Event::Down {
            name: down_backup.clone(),
            index: idx1,
        })
        .await
        .expect("send event failed");
    wait_for_events(&handle, before).await;

    // ACT
    event_tx
        .send(Event::Deleted {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&healthy_backup)
    })
    .await;
    assert!(
        status.backups.contains(&down_backup),
        "a carrier-less backup must stay a backup instead of being promoted"
    );
    drop(event_tx);
}

#[tokio::test]
async fn primary_deletion_without_an_eligible_backup_degrades() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], false);
    let primary_name = Name::new("eth0").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    event_tx
        .send(Event::Deleted {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| status.primary.is_none()).await;
    assert_eq!(
        status.state,
        supervisor::NetworkState::Degraded,
        "removing the primary without an eligible backup must degrade the network"
    );
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

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Degraded
    })
    .await;
    assert!(
        status.primary.is_none(),
        "a removed primary must not stay elected"
    );
    assert!(
        status.owner.is_none(),
        "no interface can own L3 without a primary"
    );
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

    let name1 = Name::new("eth1").expect("valid name");
    event_tx
        .send(Event::Deleted {
            name: name1,
            index: idx1,
        })
        .await
        .expect("send");

    // ASSERT
    let status = wait_for(&handle, |status| status.primary.is_none()).await;
    assert_eq!(status.state, supervisor::NetworkState::Degraded);
    drop(event_tx);
}

#[tokio::test]
async fn repeated_failover_and_recovery_converges() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);

    let (event_tx, event_rx) = mpsc::channel(64);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    // ACT
    for _ in 0_i32..5_i32 {
        drop(
            event_tx
                .send(Event::Down {
                    name: primary_name.clone(),
                    index: idx0,
                })
                .await,
        );
        drop(
            event_tx
                .send(Event::Up {
                    name: primary_name.clone(),
                    index: idx0,
                })
                .await,
        );
    }

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Operational
            && status.primary.as_ref() == Some(&primary_name)
    })
    .await;
    assert_eq!(status.backups, vec![backup_name]);
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
    let before = handle
        .status()
        .await
        .expect("the supervisor must stay responsive")
        .processed_events;
    event_tx
        .send(Event::Up {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");
    let status = wait_for_events(&handle, before).await;

    // ASSERT
    assert_eq!(
        status.state,
        supervisor::NetworkState::Ready,
        "a link-up while ready must not change readiness"
    );
    assert_eq!(status.primary, Some(name));
    drop(event_tx);
}

#[tokio::test]
async fn primary_recovery_restores_operational() {
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
            name: primary_name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert_eq!(
        status.backups,
        vec![primary_name.clone()],
        "the failed primary becomes the only backup"
    );

    event_tx
        .send(Event::Down {
            name: backup_name.clone(),
            index: idx1,
        })
        .await
        .expect("send failed");
    // Orders the recovery event after the failure it has to undo.
    let before = status.processed_events;
    let degraded = wait_for_events(&handle, before).await;
    assert_eq!(
        degraded.state,
        supervisor::NetworkState::Degraded,
        "the last uplink going down must degrade the network"
    );
    assert_eq!(
        degraded.primary,
        Some(backup_name.clone()),
        "the only uplink stays elected while it is down"
    );

    event_tx
        .send(Event::Up {
            name: backup_name.clone(),
            index: idx1,
        })
        .await
        .expect("send failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Operational
            && status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert!(
        status.backups.contains(&primary_name),
        "the failed primary must remain a backup"
    );
    drop(event_tx);
}

#[tokio::test]
async fn double_link_down_keeps_the_primary() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], false);

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let name = Name::new("eth0").expect("valid name");

    // ACT
    let before = handle
        .status()
        .await
        .expect("the supervisor must stay responsive")
        .processed_events;
    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");
    let status = wait_for_events(&handle, before).await;

    event_tx
        .send(Event::Down {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");
    let before = status.processed_events;
    wait_for_events(&handle, before).await;

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Degraded
    })
    .await;
    assert_eq!(
        status.primary,
        Some(name),
        "a repeated link-down must not clear the primary"
    );
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
    wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Degraded
    })
    .await;

    event_tx
        .send(Event::Deleted {
            name: name.clone(),
            index: idx0,
        })
        .await
        .expect("send failed");

    // ASSERT
    let status = wait_for(&handle, |status| status.primary.is_none()).await;
    assert_eq!(status.state, supervisor::NetworkState::Degraded);
    drop(event_tx);
}

#[tokio::test]
async fn primary_failure_rebridges_the_backup_port() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let br_index = link_index(&mock, "br0").expect("the bridge must exist");
    let static_ipv4 = (core::net::Ipv4Addr::new(10, 0, 0, 2), 24);
    mock.add_link("tap-0", [0xDD; 6], true);
    mock.enslave("tap-0", "br0");
    assert_eq!(
        master_index(&mock, "eth0"),
        Some(br_index),
        "the initial primary must be the bridge port"
    );
    assert!(
        ipv4_addrs(&mock, br_index).contains(&static_ipv4),
        "the bridge must hold the static IPv4 before failover"
    );

    // ACT
    mock.set_carrier("eth0", false);
    event_tx
        .send(Event::Down {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
            && status.state == supervisor::NetworkState::Operational
    })
    .await;
    assert_eq!(
        master_index(&mock, "eth1"),
        Some(br_index),
        "the promoted backup must become the bridge port"
    );
    assert_eq!(
        master_index(&mock, "eth0"),
        None,
        "the failed port must be detached from the bridge"
    );
    assert!(
        ipv4_addrs(&mock, br_index).contains(&static_ipv4),
        "the bridge must keep the same IPv4 after being re-pointed"
    );
    assert_eq!(
        master_index(&mock, "tap-0"),
        Some(br_index),
        "a foreign bridge port must stay attached across failover"
    );
}

#[tokio::test]
async fn deleting_the_bridge_port_reports_the_owner_down() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let status = handle.status().await.expect("status query failed");
    assert_eq!(
        status.owner_link,
        Some(State::Up),
        "the bridge must start with the port's carrier"
    );

    // ACT
    mock.remove_link("eth0");
    event_tx
        .send(Event::Deleted {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.owner_link == Some(State::Down) && status.state == supervisor::NetworkState::Degraded
    })
    .await;
    assert_eq!(
        status.owner,
        Some(Name::new("br0").expect("valid name")),
        "the configured bridge must keep owning L3 after its last port is removed"
    );
}

#[tokio::test]
async fn deleting_the_bridge_port_with_a_backup_keeps_the_owner_up() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    let br_index = link_index(&mock, "br0").expect("the bridge must exist");

    // ACT
    mock.remove_link("eth0");
    event_tx
        .send(Event::Deleted {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert_eq!(
        status.owner,
        Some(Name::new("br0").expect("valid name")),
        "the bridge must keep owning L3 after being re-pointed"
    );
    assert_eq!(
        status.owner_link,
        Some(State::Up),
        "re-pointing the bridge at a live port must keep the owner up"
    );
    assert_eq!(
        master_index(&mock, "eth1"),
        Some(br_index),
        "the surviving backup must become the bridge port"
    );
    assert_eq!(
        master_index(&mock, "eth0"),
        None,
        "the deleted port must not stay attached to the bridge"
    );
}

#[tokio::test]
async fn repointing_the_bridge_keeps_the_owner_up() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");
    let backup_name = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    mock.set_carrier("eth0", false);
    event_tx
        .send(Event::Down {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");

    // ASSERT
    let status = wait_for(&handle, |status| {
        status.primary.as_ref() == Some(&backup_name)
    })
    .await;
    assert_eq!(
        status.owner_link,
        Some(State::Up),
        "re-pointing the bridge at a live port must keep the owner up"
    );
}

#[tokio::test]
async fn port_flap_with_a_remaining_port_keeps_the_owner_up() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    let idx0 = mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    let primary_name = Name::new("eth0").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    mock.enslave("eth1", "br0");
    let status = handle.status().await.expect("status query failed");
    assert_eq!(
        status.owner_link,
        Some(State::Up),
        "the bridge must start up with two live ports"
    );
    let before = status.processed_events;

    // ACT
    mock.set_carrier("eth0", false);
    event_tx
        .send(Event::Down {
            name: primary_name,
            index: idx0,
        })
        .await
        .expect("send event failed");
    let status = wait_for_events(&handle, before).await;

    // ASSERT
    assert_eq!(
        status.owner_link,
        Some(State::Up),
        "the aggregate carrier never dropped, so the owner must stay up"
    );
    assert!(
        matches!(
            status.state,
            supervisor::NetworkState::Operational | supervisor::NetworkState::Ready
        ),
        "a port flap with a remaining port must not degrade the network, got {}",
        status.state
    );
    assert_eq!(
        status.owner,
        Some(Name::new("br0").expect("valid name")),
        "the bridge must keep owning L3"
    );
    drop(event_tx);
}

#[tokio::test]
async fn bridge_recovery_keeps_the_promoted_primary() {
    // ARRANGE
    let config = config_bridge();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);
    mock.add_link("eth1", [0xBB; 6], true);
    mock.add_link("eth2", [0xCC; 6], true);
    let bridge_name = Name::new("br0").expect("valid name");
    let first_backup = Name::new("eth1").expect("valid name");

    let (event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock.clone(), event_rx, config, Resolver::default());

    let result = timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());
    let br_index = link_index(&mock, "br0").expect("the bridge must exist");

    // ACT
    event_tx
        .send(Event::Down {
            name: bridge_name.clone(),
            index: br_index,
        })
        .await
        .expect("send event failed");
    wait_for(&handle, |status| {
        status.state == supervisor::NetworkState::Operational
            && status.owner_link == Some(State::Up)
    })
    .await;

    // ASSERT
    let promoted = handle.status().await.expect("status query failed");
    assert_eq!(
        promoted.primary,
        Some(first_backup.clone()),
        "the bridge recovering must not promote a second backup"
    );

    // ACT
    let before = promoted.processed_events;
    event_tx
        .send(Event::Up {
            name: bridge_name,
            index: br_index,
        })
        .await
        .expect("send event failed");
    let status = wait_for_events(&handle, before).await;

    // ASSERT
    assert_eq!(
        status.primary,
        Some(first_backup),
        "recovering the bridge must not change the primary"
    );
    drop(event_tx);
}
