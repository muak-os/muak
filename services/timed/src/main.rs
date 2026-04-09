//! timed - NTP time synchronization daemon.
//!
//! A small SNTP client that periodically synchronizes the system clock
//! against a configured NTP server. Uses simple direct clock setting (no PLL).

mod ntp;

use std::time::Duration;

use anyhow::{Context, bail};
use granola::Health;
use tokio::signal::unix::{SignalKind, signal};

/// Poll interval between NTP synchronizations.
const POLL_INTERVAL: Duration = Duration::from_secs(64);

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

    match ntp::sync(server).await {
        Ok(offset) => {
            println!("Initial time sync succeeded (offset: {offset:?})");
        }
        Err(e) => {
            eprintln!("Initial time sync failed: {e:#}");
            notifier.status("Initial sync failed, retrying", Health::Degraded)?;
        }
    }

    notifier.ready()?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            _ = interval.tick() => {
                match ntp::sync(server).await {
                    Ok(offset) => {
                        println!("Time sync succeeded (offset: {offset:?})");
                    }
                    Err(e) => {
                        eprintln!("Time sync failed: {e:#}");
                        notifier.status("Sync failed", Health::Degraded)?;
                    }
                }
            }
        }
    }

    Ok(())
}
