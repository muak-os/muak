//! Netlink monitor lifecycle: subscription startup, renewal, and event hand-off.

use core::future::Future;

use anyhow::Result;
use netdev::Ops;
use netlink_monitor::{Config, Event, Failure};
use tokio::sync::mpsc;

use super::{NetworkSupervisor, Source, dispatch};

const MAX_ATTEMPTS: u8 = 3;
const RETRY_DELAY_MS: u64 = 100;

/// The live netlink monitor.
pub(super) struct Live;

impl Live {
    async fn open() -> Result<mpsc::Receiver<Event>> {
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
}

impl Source for Live {
    fn subscribe(&self) -> impl Future<Output = Result<mpsc::Receiver<Event>>> + Send {
        Self::open()
    }
}

/// Starts the network event monitor, retrying until the subscription succeeds.
pub(super) async fn start() -> Result<mpsc::Receiver<Event>> {
    Live.subscribe().await
}

/// Dispatches a monitor event and renews the subscription when the stream closed.
pub(super) async fn handle_event<N: Ops, M: Source>(
    supervisor: &mut NetworkSupervisor<N>,
    event_rx: &mut mpsc::Receiver<Event>,
    source: &M,
    event: Event,
) {
    let closed = matches!(&event, Event::Error(Failure::Closed));
    dispatch::handle_event(supervisor, event).await;
    if closed {
        renew(event_rx, source).await;
    }
}

/// Renews the monitor subscription after its event stream closed. A failed renewal keeps
/// the closed receiver, so the periodic reconcile still converges known interfaces, but a
/// hot-plugged link would not be discovered until the process restarts.
pub(super) async fn renew<M: Source>(event_rx: &mut mpsc::Receiver<Event>, source: &M) {
    match source.subscribe().await {
        Ok(rx) => *event_rx = rx,
        Err(e) => kmsg::warn!("Failed to renew network event monitor: {e}"),
    }
}
