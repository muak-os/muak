//! Netlink event monitor for network interface changes.

#![warn(missing_docs)]

mod pump;

use std::io;

use netlink::link::ifname::Name;
use thiserror::Error;
use tokio::sync::mpsc;

/// Monitor operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to create netlink connection.
    #[error("failed to create netlink connection: {0}")]
    Connection(#[source] io::Error),
    /// Failed to enumerate links.
    #[error("failed to enumerate links: {0}")]
    List(#[source] rtnetlink::Error),
}

/// Monitor operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// An event emitted by the network monitor when a link changes state.
#[derive(Debug, Clone)]
pub enum Event {
    /// Interface carrier detected.
    Up {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
    /// Interface carrier lost.
    Down {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
    /// New interface added.
    Added {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
        /// MAC address.
        mac: [u8; 6],
    },
    /// Interface removed.
    Deleted {
        /// Interface name.
        name: Name,
        /// Kernel interface index.
        index: u32,
    },
}

/// Controls which categories of events the monitor emits.
pub struct Config {
    /// Whether to monitor link state changes.
    pub monitor_link_state: bool,
    /// Whether to monitor link additions and removals.
    pub monitor_link_changes: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor_link_state: true,
            monitor_link_changes: true,
        }
    }
}

/// Starts the network event monitor, returning a receiver of [`Event`] values.
///
/// # Errors
///
/// Returns an error when creating the netlink connection fails.
pub fn start(config: Config) -> Result<mpsc::Receiver<Event>> {
    let (tx, rx) = mpsc::channel(32);
    let (connection, handle, mut messages) =
        rtnetlink::new_connection().map_err(Failure::Connection)?;

    tokio::spawn(async move {
        let () = connection.await;
    });

    tokio::spawn(async move {
        pump::run(&config, &handle, &mut messages, &tx).await;
    });

    println!("Network event monitor started");
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_enables_state_and_change_events() {
        // ACT
        let config = Config::default();

        // ASSERT
        assert!(config.monitor_link_state);
        assert!(config.monitor_link_changes);
    }
}
