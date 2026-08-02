//! timed - NTP time synchronization daemon.
//!
//! A small SNTP client that periodically synchronizes the system clock
//! against a configured NTP server. Uses simple direct clock setting (no PLL).

extern crate alloc;

mod ntp;

use alloc::sync::Arc;
use core::time::Duration;

use anyhow::{Context as _, bail};
use granola::Health;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio::time::timeout;

/// Retry interval before the first successful NTP synchronization.
const INITIAL_RETRY_INTERVAL: Duration = Duration::from_secs(64);

/// Poll interval after the first successful NTP synchronization.
const STEADY_STATE_INTERVAL: Duration = Duration::from_hours(1);

#[granola::service("timed")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init().context("Failed to initialize system configuration")?;

    let server = &config::host().ntp;
    if server.is_empty() {
        bail!("host.ntp is not configured");
    }
    println!("NTP server: {server}");

    notifier.status("Initializing", Health::Healthy)?;

    let mut synced_once = false;

    match ntp::sync(server).await {
        Ok(offset) => {
            println!("Initial time sync succeeded (offset: {offset:?})");
            synced_once = true;
        }
        Err(e) => {
            eprintln!("Initial time sync failed: {e:#}");
            notifier.status("Initial sync failed, retrying", Health::Degraded)?;
        }
    }

    notifier.ready()?;

    let notify = Arc::new(Notify::new());

    let mut sigterm = signal(SignalKind::terminate())?;
    let signal_notify = Arc::clone(&notify);
    tokio::spawn(async move {
        sigterm.recv().await;
        signal_notify.notify_waiters();
    });

    let mut sigint = signal(SignalKind::interrupt())?;
    let signal_notify = Arc::clone(&notify);
    tokio::spawn(async move {
        sigint.recv().await;
        signal_notify.notify_waiters();
    });

    loop {
        let delay = if synced_once {
            STEADY_STATE_INTERVAL
        } else {
            INITIAL_RETRY_INTERVAL
        };

        if timeout(delay, notify.notified()).await.is_ok() {
            break;
        }

        match ntp::sync(server).await {
            Ok(offset) => {
                println!("Time sync succeeded (offset: {offset:?})");
                synced_once = true;
                notifier.status("Synchronized", Health::Healthy)?;
            }
            Err(e) => {
                eprintln!("Time sync failed: {e:#}");
                notifier.status("Sync failed", Health::Degraded)?;
            }
        }
    }

    Ok(())
}
