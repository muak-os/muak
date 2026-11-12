//! Time synchronization daemon.
//!
//! Sets the system clock from the configured source: the hypervisor clock
//! when available (see `host.clock`), or SNTP against a configured NTP
//! server. Uses simple direct clock setting (no PLL).

mod clock;
mod hypervisor;
mod ntp;
mod source;

use core::time::Duration;

use anyhow::{Context as _, bail};
use granola::runtime::notify::Health;
use source::Source;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::timeout;

/// Retry interval before the first successful NTP synchronization.
const INITIAL_RETRY_INTERVAL: Duration = Duration::from_secs(64);

/// Poll interval after the first successful NTP synchronization.
const STEADY_STATE_INTERVAL: Duration = Duration::from_hours(1);

#[granola::service("timed")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init().context("Failed to initialize system configuration")?;

    let host = config::system::host().context("Failed to load host configuration")?;

    let source = Source::from_config(&host.clock).context("Invalid host.clock configuration")?;
    let server = &host.ntp;
    if source.needs_ntp() && server.is_empty() {
        bail!("host.ntp is not configured");
    }
    println!("Clock source: {source}");

    notifier.status("Initializing", Health::Healthy)?;

    let mut synced_once = false;

    match source::sync(source, server).await {
        Ok((used, offset)) => {
            println!("Initial time sync succeeded via {used} (offset: {offset:?})");
            synced_once = true;
        }
        Err(e) => {
            eprintln!("Initial time sync failed: {e:#}");
            notifier.status("Initial sync failed, retrying", Health::Degraded)?;
        }
    }

    notifier.ready()?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        let delay = if synced_once {
            STEADY_STATE_INTERVAL
        } else {
            INITIAL_RETRY_INTERVAL
        };

        let shutdown = async {
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = sigint.recv() => {}
            }
        };

        if timeout(delay, shutdown).await.is_ok() {
            break;
        }

        match source::sync(source, server).await {
            Ok((used, offset)) => {
                println!("Time sync succeeded via {used} (offset: {offset:?})");
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
