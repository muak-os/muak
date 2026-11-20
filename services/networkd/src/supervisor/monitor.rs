//! Netlink monitor lifecycle: subscription startup, renewal pacing, and event hand-off.

use core::future::Future;
use core::time::Duration;

use anyhow::Result;
use netdev::Ops;
use netlink_monitor::{Config, Event, Failure};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};

use super::{NetworkSupervisor, Source, dispatch};

const MAX_ATTEMPTS: u8 = 3;
const RETRY_DELAY_MS: u64 = 100;
const RENEW_FLOOR: Duration = Duration::from_secs(1);

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

/// Owns the monitor subscription, its source, and the renewal pacing.
pub(super) struct Subscription<M> {
    rx: mpsc::Receiver<Event>,
    source: M,
    last_renewal: Option<Instant>,
}

impl<M: Source> Subscription<M> {
    pub(super) fn new(rx: mpsc::Receiver<Event>, source: M) -> Self {
        Self {
            rx,
            source,
            last_renewal: None,
        }
    }

    /// Returns the receiver, for the supervisor's event selection.
    pub(super) fn receiver(&mut self) -> &mut mpsc::Receiver<Event> {
        &mut self.rx
    }

    /// Dispatches a monitor event, renewing the subscription when its stream closed.
    pub(super) async fn handle<N: Ops>(
        &mut self,
        supervisor: &mut NetworkSupervisor<N>,
        event: Event,
    ) {
        let closed = matches!(&event, Event::Error(Failure::Closed));
        dispatch::handle_event(supervisor, event).await;
        if closed {
            self.renew().await;
        }
    }

    /// Renews the subscription when its stream is closed, used by the reconcile tick.
    pub(super) async fn retry_if_closed(&mut self) {
        if self.rx.is_closed() {
            self.renew().await;
        }
    }

    // Renews the subscription, waiting out the floor since the previous attempt.
    async fn renew(&mut self) {
        if let Some(previous) = self.last_renewal {
            sleep(RENEW_FLOOR.saturating_sub(previous.elapsed())).await;
        }
        match self.source.subscribe().await {
            Ok(rx) => self.rx = rx,
            Err(e) => kmsg::warn!("Failed to renew network event monitor: {e}"),
        }
        self.last_renewal = Some(Instant::now());
    }
}

/// Starts the network event monitor, retrying until the subscription succeeds.
pub(super) async fn start() -> Result<mpsc::Receiver<Event>> {
    Live.subscribe().await
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, PoisonError};

    use super::*;

    struct FlakySource {
        calls: AtomicU32,
        failures: AtomicU32,
        senders: Arc<Mutex<Vec<mpsc::Sender<Event>>>>,
    }

    impl FlakySource {
        fn new(failures: u32) -> Self {
            Self {
                calls: AtomicU32::new(0),
                failures: AtomicU32::new(failures),
                senders: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }

        // Drops every handed out sender, closing the current subscription.
        fn close(&self) {
            self.lock().clear();
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Vec<mpsc::Sender<Event>>> {
            self.senders.lock().unwrap_or_else(PoisonError::into_inner)
        }

        // Returns a fresh subscription, or `None` while attempts are still meant to fail.
        fn attempt(&self) -> Option<mpsc::Receiver<Event>> {
            let (tx, rx) = mpsc::channel(1);
            let succeeds =
                self.calls.fetch_add(1, Ordering::SeqCst) >= self.failures.load(Ordering::SeqCst);
            succeeds.then(|| self.lock().push(tx)).map(|()| rx)
        }
    }

    impl Source for FlakySource {
        fn subscribe(&self) -> impl Future<Output = Result<mpsc::Receiver<Event>>> + Send {
            core::future::ready(
                self.attempt()
                    .ok_or_else(|| anyhow::anyhow!("mock monitor failure")),
            )
        }
    }

    fn closed_receiver() -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::channel(1);
        drop(tx);

        rx
    }

    #[tokio::test(start_paused = true)]
    async fn retry_if_closed_renews_after_a_failed_attempt() {
        // ARRANGE
        let mut subscription = Subscription::new(closed_receiver(), FlakySource::new(1));
        subscription.retry_if_closed().await;
        assert_eq!(subscription.source.calls(), 1);
        assert!(
            subscription.receiver().is_closed(),
            "a failed renewal must keep the closed receiver"
        );

        // ACT
        subscription.retry_if_closed().await;

        // ASSERT
        assert_eq!(subscription.source.calls(), 2);
        assert!(
            !subscription.receiver().is_closed(),
            "the reconcile tick must reopen a subscription that failed to renew"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn renew_waits_at_least_the_floor_between_attempts() {
        // ARRANGE
        let mut subscription = Subscription::new(closed_receiver(), FlakySource::new(0));
        subscription.retry_if_closed().await;
        assert_eq!(subscription.source.calls(), 1);
        subscription.source.close();
        assert!(subscription.receiver().is_closed());
        let start = Instant::now();

        // ACT
        subscription.retry_if_closed().await;

        // ASSERT
        assert!(
            start.elapsed() >= RENEW_FLOOR,
            "renewals must be paced by the floor, elapsed {:?}",
            start.elapsed()
        );
        assert_eq!(subscription.source.calls(), 2);
        assert!(!subscription.receiver().is_closed());
    }
}
