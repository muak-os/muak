//! Wake deadline scheduling for the interface controller.

use std::time::Instant;

use netdev::Ops;

use super::Interface;
use crate::family::Family;
use crate::view;

pub(super) async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => core::future::pending().await,
    }
}

pub(super) fn earliest_wake<N, V4, V6>(
    interface: &Interface<N, V4, V6>,
    now: Instant,
) -> Option<Instant>
where
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
{
    let deadline = earliest(None, interface.v4.next_wake(now));

    earliest(deadline, interface.v6.next_wake(now))
}

fn earliest(current: Option<Instant>, candidate: Option<Instant>) -> Option<Instant> {
    match (current, candidate) {
        (Some(first), Some(second)) => Some(first.min(second)),
        (found, None) | (None, found) => found,
    }
}
