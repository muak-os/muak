//! Wake deadline scheduling for the interface controller.

use std::time::Instant;

use netdev::Ops;

use super::Interface;

pub(super) async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => core::future::pending().await,
    }
}

pub(super) fn earliest_wake<N: Ops>(interface: &Interface<N>, now: Instant) -> Option<Instant> {
    let deadline = None;
    #[cfg(feature = "ipv4")]
    let deadline = earliest(
        deadline,
        interface
            .v4
            .as_ref()
            .and_then(|method| method.next_wake(now)),
    );
    #[cfg(feature = "ipv6")]
    let deadline = earliest(
        deadline,
        interface
            .v6
            .as_ref()
            .and_then(|method| method.next_wake(now)),
    );

    deadline
}

fn earliest(current: Option<Instant>, candidate: Option<Instant>) -> Option<Instant> {
    match (current, candidate) {
        (Some(first), Some(second)) => Some(first.min(second)),
        (found, None) | (None, found) => found,
    }
}
