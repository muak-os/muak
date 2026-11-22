//! Family-neutral interface controller.

mod command;
mod event;
mod intent;
mod notice;
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

use crate::view;

/// Commands the supervisor sends to an interface controller.
#[derive(Debug)]
pub enum Command {
    /// Apply the given per-family configurations.
    Configure {
        /// IPv4 configuration to apply.
        #[cfg(feature = "ipv4")]
        v4: Option<addrconf::v4::Config>,
        /// IPv6 configuration to apply.
        #[cfg(feature = "ipv6")]
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
    /// IPv4 method view, when enabled and configured.
    #[cfg(feature = "ipv4")]
    pub v4: Option<view::V4State>,
    /// IPv4 provisioning phase.
    #[cfg(feature = "ipv4")]
    pub v4_state: State,
    /// IPv6 method view, when enabled and configured.
    #[cfg(feature = "ipv6")]
    pub v6: Option<view::V6State>,
    /// IPv6 provisioning phase.
    #[cfg(feature = "ipv6")]
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
#[cfg(feature = "ipv4")]
pub type V4Notice = Notice<view::V4State>;

/// IPv6 transition notice.
#[cfg(feature = "ipv6")]
pub type V6Notice = Notice<view::V6State>;

/// Transition notice receivers, one per enabled family.
pub struct Notices {
    /// IPv4 transition notices.
    #[cfg(feature = "ipv4")]
    pub v4: mpsc::Receiver<V4Notice>,
    /// IPv6 transition notices.
    #[cfg(feature = "ipv6")]
    pub v6: mpsc::Receiver<V6Notice>,
}

/// Family-neutral controller for one interface.
pub struct Interface<N: Ops> {
    link: Ethernet,
    ops: N,
    #[cfg(feature = "ipv4")]
    v4: Option<v4::Method>,
    #[cfg(feature = "ipv6")]
    v6: Option<v6::Method>,
    snapshot_tx: watch::Sender<Snapshot>,
    command_rx: mpsc::Receiver<Command>,
    #[cfg(feature = "ipv4")]
    v4_state: State,
    #[cfg(feature = "ipv6")]
    v6_state: State,
    #[cfg(feature = "ipv4")]
    v4_notice_tx: mpsc::Sender<V4Notice>,
    #[cfg(feature = "ipv6")]
    v6_notice_tx: mpsc::Sender<V6Notice>,
    #[cfg(feature = "ipv4")]
    v4_configured: bool,
    #[cfg(feature = "ipv6")]
    v6_configured: bool,
}

impl<N: Ops> Interface<N> {
    /// Spawns a controller for the given link and returns its handle and notices.
    pub fn spawn(ops: N, link: Ethernet) -> (Handle, Notices) {
        let snapshot = Snapshot {
            link: link.link_state,
            #[cfg(feature = "ipv4")]
            v4: None,
            #[cfg(feature = "ipv4")]
            v4_state: State::Stopped,
            #[cfg(feature = "ipv6")]
            v6: None,
            #[cfg(feature = "ipv6")]
            v6_state: State::Stopped,
        };
        let (command_tx, command_rx) = mpsc::channel(32);
        let (snapshot_tx, snapshot_rx) = watch::channel(snapshot);
        #[cfg(feature = "ipv4")]
        let (v4_notice_tx, v4_notice_rx) = mpsc::channel(32);
        #[cfg(feature = "ipv6")]
        let (v6_notice_tx, v6_notice_rx) = mpsc::channel(32);

        let controller = Self {
            link,
            ops,
            #[cfg(feature = "ipv4")]
            v4: None,
            #[cfg(feature = "ipv6")]
            v6: None,
            snapshot_tx,
            command_rx,
            #[cfg(feature = "ipv4")]
            v4_state: State::Stopped,
            #[cfg(feature = "ipv6")]
            v6_state: State::Stopped,
            #[cfg(feature = "ipv4")]
            v4_notice_tx,
            #[cfg(feature = "ipv6")]
            v6_notice_tx,
            #[cfg(feature = "ipv4")]
            v4_configured: false,
            #[cfg(feature = "ipv6")]
            v6_configured: false,
        };
        tokio::spawn(command::run(controller));

        (
            Handle {
                command_tx,
                snapshot_rx,
            },
            Notices {
                #[cfg(feature = "ipv4")]
                v4: v4_notice_rx,
                #[cfg(feature = "ipv6")]
                v6: v6_notice_rx,
            },
        )
    }
}
