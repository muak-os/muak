//! Family-neutral interface controller.

mod command;
mod event;
mod intent;
#[cfg(feature = "ipv4")]
pub mod v4;
#[cfg(feature = "ipv6")]
pub mod v6;
mod wake;

use alloc::string::String;

use addrconf::State;
use netdev::Ops;
use netdev::interface::Ethernet;
use netlink::link::state::State as LinkState;
use tokio::sync::{mpsc, watch};

use crate::family::{Family, Slot};
use crate::view;

/// Commands the supervisor sends to an interface controller.
#[derive(Debug)]
pub enum Command {
    /// Apply the given per-family configurations.
    Configure {
        /// IPv4 configuration to apply.
        v4: Option<addrconf::v4::Config>,
        /// IPv6 configuration to apply.
        v6: Option<addrconf::v6::Config>,
    },
    /// Reconcile desired and actual configuration.
    Reconcile,
    /// Bring the link administratively up.
    LinkUp,
    /// Bring the link administratively down.
    LinkDown,
    /// Stop every method and shut the controller down.
    Shutdown,
}

/// Point-in-time state of an interface and its address methods.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Administrative and carrier state of the link.
    pub link: LinkState,
    /// IPv4 method view, when configured.
    pub v4: Option<view::V4State>,
    /// IPv4 provisioning phase.
    pub v4_state: State,
    /// IPv6 method view, when configured.
    pub v6: Option<view::V6State>,
    /// IPv6 provisioning phase.
    pub v6_state: State,
}

/// Handle used to drive and observe an interface controller.
pub struct Handle {
    /// Channel carrying controller commands.
    pub command_tx: mpsc::Sender<Command>,
    /// Channel publishing interface snapshots.
    pub snapshot_rx: watch::Receiver<Snapshot>,
}

/// Transitional notice emitted by an interface controller.
pub enum Notice<V> {
    /// Method lifecycle state changed.
    State(State),
    /// Family configuration became available.
    Configured(V),
    /// Non-fatal family warning.
    Warning(String),
}

/// IPv4 transition notice.
pub type V4Notice = Notice<view::V4State>;

/// IPv6 transition notice.
pub type V6Notice = Notice<view::V6State>;

/// Transition notice receivers, one per family.
pub struct Notices {
    /// IPv4 transition notices.
    pub v4: mpsc::Receiver<V4Notice>,
    /// IPv6 transition notices.
    pub v6: mpsc::Receiver<V6Notice>,
}

/// Family-generic controller for one interface.
pub struct Interface<
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
> {
    link: Ethernet,
    ops: N,
    v4: Slot<V4>,
    v6: Slot<V6>,
    snapshot_tx: watch::Sender<Snapshot>,
    command_rx: mpsc::Receiver<Command>,
}

impl<
    N: Ops,
    V4: Family<Config = addrconf::v4::Config, View = view::V4State>,
    V6: Family<Config = addrconf::v6::Config, View = view::V6State>,
> Interface<N, V4, V6>
{
    /// Spawns a controller for the given link and returns its handle and notices.
    pub fn spawn(ops: N, link: Ethernet) -> (Handle, Notices) {
        let snapshot = Snapshot {
            link: link.link_state,
            v4: None,
            v4_state: State::Stopped,
            v6: None,
            v6_state: State::Stopped,
        };
        let (command_tx, command_rx) = mpsc::channel(32);
        let (snapshot_tx, snapshot_rx) = watch::channel(snapshot);
        let (v4_notice_tx, v4_notice_rx) = mpsc::channel(32);
        let (v6_notice_tx, v6_notice_rx) = mpsc::channel(32);

        let controller = Self {
            link,
            ops,
            v4: Slot::new(v4_notice_tx),
            v6: Slot::new(v6_notice_tx),
            snapshot_tx,
            command_rx,
        };
        tokio::spawn(command::run(controller));

        (
            Handle {
                command_tx,
                snapshot_rx,
            },
            Notices {
                v4: v4_notice_rx,
                v6: v6_notice_rx,
            },
        )
    }
}
