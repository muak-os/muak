//! Family-neutral interface controller.
//!
//! The controller owns one interface's link state, its active address methods,
//! and publishes a [`Snapshot`] whenever anything changes.

mod event;
mod intent;
#[cfg(feature = "ipv4")]
pub mod v4;
#[cfg(feature = "ipv6")]
pub mod v6;

use netdev::Ops;
use netdev::interface::Ethernet;
use netlink::link::state::State as LinkState;
use provision::State;
use tokio::sync::{mpsc, watch};

use crate::{provision, view};

/// Commands the supervisor sends to an interface controller.
#[derive(Debug)]
pub enum Command {
    /// Apply the given per-family configurations.
    Configure {
        /// IPv4 configuration to apply.
        #[cfg(feature = "ipv4")]
        v4: Option<provision::v4::Config>,
        /// IPv6 configuration to apply.
        #[cfg(feature = "ipv6")]
        v6: Option<provision::v6::Config>,
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
}

impl<N: Ops> Interface<N> {
    /// Spawns a controller for the given link and returns its handle.
    pub fn spawn(ops: N, link: Ethernet) -> Handle {
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
        };
        tokio::spawn(controller.run());

        Handle {
            command_tx,
            snapshot_rx,
        }
    }

    async fn run(mut self) {
        while let Some(event) = event::select(&mut self).await {
            self.handle(event).await;
        }
    }

    async fn handle(&mut self, selected: event::Selected) {
        match selected {
            event::Selected::Command(command) => self.dispatch(command).await,
            #[cfg(feature = "ipv4")]
            event::Selected::V4(event) => event::handle_v4(self, event).await,
            #[cfg(feature = "ipv6")]
            event::Selected::V6(event) => event::handle_v6(self, event).await,
        }
    }

    async fn dispatch(&mut self, command: Command) {
        match command {
            Command::Configure {
                #[cfg(feature = "ipv4")]
                v4,
                #[cfg(feature = "ipv6")]
                v6,
            } => {
                #[cfg(feature = "ipv4")]
                self.configure_v4(v4).await;
                #[cfg(feature = "ipv6")]
                event::configure_v6(self, v6);
                self.refresh_index().await;
                self.publish();
            }
            Command::Reconcile => self.forward(provision::Command::Reconcile),
            Command::LinkUp => {
                self.link.link_state = LinkState::Up;
                self.forward(provision::Command::LinkUp);
                self.publish();
            }
            Command::LinkDown => {
                self.link.link_state = LinkState::Down;
                self.forward(provision::Command::LinkDown);
                self.publish();
            }
            Command::Shutdown => {
                self.stop_families();
                self.publish();
                self.command_rx.close();
            }
        }
    }

    #[cfg(feature = "ipv4")]
    async fn configure_v4(&mut self, config: Option<provision::v4::Config>) {
        let Some(config) = config else {
            return;
        };
        match v4::Method::from_config(config, &self.link).await {
            Ok(mut method) => {
                method.on_command(provision::Command::Start);
                self.v4 = Some(method);
            }
            Err(failure) => {
                kmsg::warn!(
                    "netstack: IPv4 method setup failed on {}: {failure}",
                    self.link.name
                );
                self.v4 = None;
                self.v4_state = State::Degraded;
            }
        }
    }

    fn stop_families(&mut self) {
        #[cfg(feature = "ipv4")]
        {
            self.v4 = None;
            self.v4_state = State::Stopped;
        }
        #[cfg(feature = "ipv6")]
        {
            self.v6 = None;
            self.v6_state = State::Stopped;
        }
    }

    fn forward(&mut self, command: provision::Command) {
        #[cfg(feature = "ipv4")]
        if let Some(method) = self.v4.as_mut() {
            method.on_command(command);
        }
        #[cfg(feature = "ipv6")]
        if let Some(method) = self.v6.as_mut() {
            method.on_command(command);
        }
    }

    async fn refresh_index(&mut self) {
        match self.ops.ensure_up(self.link.name.as_str()).await {
            Ok(index) => self.link.index = index,
            Err(error) => {
                kmsg::warn!("netstack: ensure link {} is up: {error}", self.link.name);
            }
        }
    }

    fn publish(&self) {
        let snapshot = Snapshot {
            link: self.link.link_state,
            #[cfg(feature = "ipv4")]
            v4: self.v4.as_ref().and_then(v4::Method::view),
            #[cfg(feature = "ipv4")]
            v4_state: self.v4_state,
            #[cfg(feature = "ipv6")]
            v6: self.v6.as_ref().and_then(v6::Method::view),
            #[cfg(feature = "ipv6")]
            v6_state: self.v6_state,
        };
        drop(self.snapshot_tx.send(snapshot));
    }
}
