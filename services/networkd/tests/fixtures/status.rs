//! Supervisor status polling for integration tests.

use core::time::Duration;

use networkd::supervisor::{NetworkActorHandle, Status};
use tokio::time::{Instant, sleep};

const POLL_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_PAUSE: Duration = Duration::from_millis(20);

/// Polls the supervisor until `predicate` holds, panicking with the last status seen.
///
/// # Panics
///
/// Panics when the supervisor is gone, or when the deadline passes before
/// `predicate` holds.
#[expect(
    clippy::expect_used,
    reason = "a closed supervisor channel is terminal, so retrying cannot help. Panicking in tests is fine."
)]
pub(crate) async fn wait_for<F>(handle: &NetworkActorHandle, predicate: F) -> Status
where
    F: Fn(&Status) -> bool,
{
    let start = Instant::now();
    loop {
        let status = handle
            .status()
            .await
            .expect("the supervisor must stay responsive");
        if predicate(&status) {
            return status;
        }
        assert!(
            start.elapsed() < POLL_TIMEOUT,
            "network never reached the expected state; last status: {status:?}"
        );
        sleep(POLL_PAUSE).await;
    }
}

/// Waits until the supervisor has dispatched more than `previous` netlink events.
pub(crate) async fn wait_for_events(handle: &NetworkActorHandle, previous: u64) -> Status {
    wait_for(handle, |status| status.processed_events > previous).await
}
