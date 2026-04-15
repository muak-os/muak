//! Integration tests for the supervisor's life cycle.

use std::time::Duration;

use super::*;

#[tokio::test]
async fn supervisor_initializes_with_one_interface() {
    // ARRANGE
    let config = super::make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config,
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    let result2 =
        tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(result2.is_ok(), "re-initialization should not time out");
    assert!(
        result2.expect("timeout").is_ok(),
        "re-initialization should succeed"
    );
}

#[tokio::test]
async fn supervisor_shuts_down_when_all_channels_dropped() {
    // ARRANGE
    let config = super::make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
    let handle = supervisor::start_with(
        mock,
        Some(event_rx),
        config,
        networkd::dns::DnsState::default(),
    )
    .expect("start failed");

    let result = tokio::time::timeout(Duration::from_secs(5), handle.initialize_with_retry()).await;
    assert!(result.is_ok() && result.expect("timeout").is_ok());

    // ACT
    drop(handle);
    drop(event_tx);

    // ASSERT
    tokio::time::sleep(Duration::from_millis(200)).await;
}
