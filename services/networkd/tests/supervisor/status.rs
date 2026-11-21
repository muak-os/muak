//! Integration tests for the supervisor status query.

use core::time::Duration;

use netlink::link::ifname::Name;
use networkd::dns::Resolver;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;

#[tokio::test]
async fn status_reports_ready_after_initialize() {
    // ARRANGE
    let config = make_config();
    let mock = MockNetlinkOps::new();
    mock.add_link("eth0", [0xAA; 6], true);

    let (_event_tx, event_rx) = mpsc::channel(32);
    let handle = supervisor::start_with(mock, event_rx, config, Resolver::default());

    let initial = handle.status().await.expect("status query should succeed");
    assert_eq!(initial.state, supervisor::NetworkState::Uninitialized);
    assert!(initial.primary.is_none());
    assert!(initial.owner.is_none());
    assert!(
        initial.backups.is_empty(),
        "a fresh supervisor elects no backups"
    );

    // ACT
    timeout(Duration::from_secs(5), handle.initialize_with_retry())
        .await
        .expect("initialization should not time out")
        .expect("initialization should succeed");

    let status = timeout(Duration::from_secs(5), handle.status())
        .await
        .expect("status query should not time out")
        .expect("status query should succeed");

    // ASSERT
    assert_eq!(status.state, supervisor::NetworkState::Ready);
    assert_eq!(status.primary, Some(Name::new("eth0").expect("valid name")));
    assert_eq!(status.owner, Some(Name::new("eth0").expect("valid name")));
    assert!(
        status.backups.is_empty(),
        "no other interface can take over as uplink"
    );
}
