//! Supervisor status polling for integration tests.

use core::time::Duration;

use networkd::supervisor::{NetworkActorHandle, Status};
use tokio::time::{Instant, sleep};

const POLL_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_PAUSE: Duration = Duration::from_millis(20);

/// Polls the supervisor until `predicate` holds, panicking with the last status seen.
///
/// The pause between polls is required: the supervisor polls its command
/// channel before netlink events, so a tight loop would starve event dispatch.
pub(crate) async fn wait_for<F>(handle: &NetworkActorHandle, predicate: F) -> Status
where
    F: Fn(&Status) -> bool,
{
    let start = Instant::now();
    loop {
        let Ok(status) = handle.status().await else {
            assert!(
                start.elapsed() < POLL_TIMEOUT,
                "the supervisor stopped before the network reached the expected state"
            );
            sleep(POLL_PAUSE).await;
            continue;
        };
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
