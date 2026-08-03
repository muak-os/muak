//! Integration tests for interface discovery edge cases.

use core::time::Duration;

use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn no_interfaces_retries_on_empty_discovery() {
    // ARRANGE
    let config = super::make_config();
    let mock = MockNetlinkOps::new();

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config, Resolver::default())
        .expect("start failed");

    // ACT
    let result = timeout(Duration::from_secs(3), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(
        result.is_err(),
        "init should not succeed when no interfaces exist"
    );
}

#[tokio::test]
async fn all_no_carrier_retries_on_degraded_discovery() {
    // ARRANGE
    let config = super::make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], false);
    mock.add_link("eth1", [0xBB; 6], false);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, Some(event_rx), config, Resolver::default())
        .expect("start failed");

    // ACT
    let result = timeout(Duration::from_secs(3), handle.initialize_with_retry()).await;

    // ASSERT
    assert!(
        result.is_err(),
        "init should not succeed when no interfaces have carrier"
    );
}
