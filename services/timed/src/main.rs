//! Time synchronization daemon.
//!
//! Sets the system clock from the configured source: the hypervisor clock
//! when available (see `host.clock`), or SNTP against a configured NTP
//! server. Uses simple direct clock setting (no PLL).

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "service output is captured per service by PID 1; the logging facade is pending"
)]

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

// First retry interval before the first successful NTP synchronization.
const INITIAL_RETRY_START: Duration = Duration::from_secs(1);

// Maximum retry interval before the first successful NTP synchronization.
const INITIAL_RETRY_MAX: Duration = Duration::from_secs(30);

// Poll interval after the first successful NTP synchronization.
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
    let mut retry = INITIAL_RETRY_START;

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
            retry
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
                retry = next_retry(retry);
            }
        }
    }

    Ok(())
}

fn next_retry(current: Duration) -> Duration {
    current.saturating_mul(2).min(INITIAL_RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::next_retry;

    #[test]
    fn retry_doubles_the_current_delay() {
        // ARRANGE
        let current = Duration::from_secs(1);

        // ACT
        let next = next_retry(current);

        // ASSERT
        assert_eq!(next, Duration::from_secs(2));
    }

    #[test]
    fn retry_caps_the_doubled_delay() {
        // ARRANGE
        let current = Duration::from_secs(16);

        // ACT
        let next = next_retry(current);

        // ASSERT
        assert_eq!(next, Duration::from_secs(30));
    }

    #[test]
    fn retry_stays_at_the_cap() {
        // ARRANGE
        let current = Duration::from_secs(30);

        // ACT
        let next = next_retry(current);

        // ASSERT
        assert_eq!(next, Duration::from_secs(30));
    }
}
