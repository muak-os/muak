//! Netlink event monitor for network interface changes.

#![warn(missing_docs)]

mod pump;
mod scan;

use core::num::NonZeroI32;
use std::collections::HashMap;
use std::io;

use netlink::Rtnl;
use netlink::link::ifname::Name;
use rtnetlink::MulticastGroup;
use thiserror::Error;
use tokio::sync::mpsc;

/// Monitor operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to create netlink connection.
    #[error("failed to create netlink connection: {0}")]
    Connection(#[source] io::Error),
    /// Failed to enumerate links during the initial scan.
    #[error("failed to enumerate links: {0}")]
    List(#[source] netlink::link::Failure),
    /// Netlink notification buffer overran, so link events may have been lost.
    #[error("netlink notification buffer overran, events may be lost")]
    Overrun,
    /// Netlink reported an error on the event socket.
    #[error("netlink event socket error: {0}")]
    Kernel(NonZeroI32),
    /// Netlink event stream closed, so link events are no longer delivered.
    #[error("netlink event stream closed")]
    Closed,
}

/// Monitor operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// An event emitted by the network monitor.
#[derive(Debug)]
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
    /// Monitor failure that may have interrupted link events.
    Error(Failure),
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
/// Returns an error when creating the netlink connection fails or when the
/// initial link scan fails.
pub async fn start(config: Config) -> Result<mpsc::Receiver<Event>> {
    let (tx, rx) = mpsc::channel(32);
    let (connection, handle, mut messages) =
        rtnetlink::new_multicast_connection(&[MulticastGroup::Link])
            .map_err(Failure::Connection)?;

    tokio::spawn(async move {
        let () = connection.await;
    });

    let rtnl = Rtnl::new(handle);
    let mut link_states: HashMap<u32, bool> = HashMap::new();
    scan::state(&rtnl, &mut link_states).await?;

    tokio::spawn(async move {
        pump::run(&config, &mut messages, &tx, link_states).await;
    });

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

    #[test]
    fn failure_variants_render_their_context() {
        // ARRANGE
        let overrun = Failure::Overrun;
        let closed = Failure::Closed;
        let kernel = Failure::Kernel(NonZeroI32::new(105_i32).expect("non-zero"));
        let list = Failure::List(netlink::link::Failure::NotFound("eth9".to_owned()));

        // ACT
        let overrun_text = overrun.to_string();
        let closed_text = closed.to_string();
        let kernel_text = kernel.to_string();
        let list_text = list.to_string();

        // ASSERT
        assert_eq!(
            overrun_text,
            "netlink notification buffer overran, events may be lost"
        );
        assert_eq!(closed_text, "netlink event stream closed");
        assert_eq!(kernel_text, "netlink event socket error: 105");
        assert!(
            list_text.starts_with("failed to enumerate links: "),
            "got {list_text}"
        );
    }
}
