/// Fake monitor source fixture for supervisor tests.
extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use anyhow::Result;
use netlink_monitor::Event;
use networkd::supervisor::Source;
use tokio::sync::mpsc;

const CHANNEL_CAPACITY: usize = 32;

/// Counts subscriptions and hands out controlled event channels.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeMonitor {
    subscribe_calls: Arc<AtomicU32>,
    senders: Arc<Mutex<Vec<mpsc::Sender<Event>>>>,
}

impl FakeMonitor {
    /// Returns a fake that has never been subscribed.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns how many times `subscribe` has been invoked.
    #[must_use = "the test asserts on how often the monitor was subscribed"]
    pub(crate) fn subscribe_calls(&self) -> u32 {
        self.subscribe_calls.load(Ordering::SeqCst)
    }

    /// Sends an event on the most recent subscription, returning whether it was queued.
    pub(crate) fn send(&self, event: Event) -> bool {
        self.lock()
            .last()
            .is_some_and(|tx| tx.try_send(event).is_ok())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<mpsc::Sender<Event>>> {
        self.senders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Source for FakeMonitor {
    fn subscribe(&self) -> impl Future<Output = Result<mpsc::Receiver<Event>>> + Send {
        self.subscribe_calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        self.lock().push(tx);

        std::future::ready(Ok(rx))
    }
}
