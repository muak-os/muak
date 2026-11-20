//! Netlink monitor lifecycle: subscription startup, renewal, and event hand-off.

use anyhow::Result;
use netdev::Ops;
use netlink_monitor::{Config, Event, Failure};
use tokio::sync::mpsc;

use super::{NetworkSupervisor, dispatch};

const MAX_ATTEMPTS: u8 = 3;
const RETRY_DELAY_MS: u64 = 100;

/// Starts the network event monitor, retrying until the subscription succeeds.
pub(super) async fn start() -> Result<mpsc::Receiver<Event>> {
    let rx = netlink::retry::run(
        || netlink_monitor::start(Config::default()),
        MAX_ATTEMPTS,
        RETRY_DELAY_MS,
        "failed to start network event monitor",
    )
    .await?;
    println!("Network event monitoring enabled");

    Ok(rx)
}

/// Dispatches a monitor event and renews the subscription when the stream closed.
pub(super) async fn handle_event<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    event_rx: &mut mpsc::Receiver<Event>,
    event: Event,
) {
    let closed = matches!(&event, Event::Error(Failure::Closed));
    dispatch::handle_event(supervisor, event).await;
    if closed {
        renew(event_rx).await;
    }
}

/// Renews the monitor subscription after its event stream closed. A failed renewal keeps
/// the closed receiver, so the periodic reconcile still converges known interfaces, but a
/// hot-plugged link would not be discovered until the process restarts.
pub(super) async fn renew(event_rx: &mut mpsc::Receiver<Event>) {
    match start().await {
        Ok(rx) => *event_rx = rx,
        Err(e) => kmsg::warn!("Failed to renew network event monitor: {e}"),
    }
}
