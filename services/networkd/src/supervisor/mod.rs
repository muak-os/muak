//! Network supervisor that routes events, drives interface controllers, and aggregates state.

mod carrier;
mod commands;
mod discovery;
mod dispatch;
mod dns;
mod failover;
mod monitor;
mod owner;
mod provision;
mod reconcile;
mod retry;
mod selection;
mod snapshot;
mod view;

use alloc::sync::Arc;
use core::fmt;
use core::time::Duration;
use std::collections::HashMap;

use anyhow::Result;
use commands::SupervisorCommand;
use config::system::network;
use netdev::Ops;
use netdev::interface::Ethernet;
use netlink::Rtnl;
use netlink::link::ifname::Name;
use netlink::link::state::State;
use netlink_monitor::Event;
use netstack::interface::{Command, Handle, Interface};
use selection::SupervisorEvent;
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

use crate::dns::Resolver;
use crate::statemachine::StateMachine;
use crate::supervisor::snapshot::NetworkSnapshot;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

/// Status of the entire network subsystem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetworkState {
    Uninitialized,
    Initializing,
    Operational,
    Ready,
    Degraded,
}

impl StateMachine for NetworkState {
    fn valid_next_states(&self) -> &'static [Self] {
        match *self {
            Self::Uninitialized => &[Self::Initializing],
            Self::Initializing => &[Self::Operational, Self::Degraded],
            Self::Operational => &[Self::Ready, Self::Degraded],
            Self::Ready => &[Self::Degraded],
            Self::Degraded => &[Self::Initializing, Self::Operational],
        }
    }
}

impl fmt::Display for NetworkState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Self::Uninitialized => "Uninitialized",
            Self::Initializing => "Initializing",
            Self::Operational => "Operational",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
        })
    }
}

/// Point-in-time status of the network supervisor.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// Global readiness.
    pub state: NetworkState,
    /// Elected uplink, i.e. the interface `auto` resolves to.
    pub primary: Option<Name>,
    /// Interface that currently owns L3: the bridge once it has been
    /// provisioned, otherwise the primary.
    pub owner: Option<Name>,
    /// Link state reported by the interface that owns L3.
    pub owner_link: Option<State>,
    /// Uplink candidates in failover preference order; entries are not
    /// guaranteed to be currently eligible.
    pub backups: Vec<Name>,
    /// Netlink events dispatched since the supervisor started.
    pub processed_events: u64,
}

struct NetworkSupervisor<N: Ops> {
    ops: N,
    config: Arc<network::Config>,
    state: NetworkSnapshot,
    interfaces: HashMap<Name, Handle>,
    owners: HashMap<Name, Name>,
    dns: Resolver,
    processed_events: u64,
}

#[derive(Clone)]
#[must_use = "dropping it shuts the supervisor down"]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<SupervisorCommand>,
}

impl<N: Ops> NetworkSupervisor<N> {
    fn new(ops: N, config: Arc<network::Config>, dns: Resolver) -> Self {
        Self {
            ops,
            config,
            state: NetworkSnapshot::empty(),
            interfaces: HashMap::new(),
            owners: HashMap::new(),
            dns,
            processed_events: 0,
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        kmsg::info!("Initializing network");

        self.reset().await;
        discovery::interfaces(self).await?;
        provision::interfaces(self).await;

        self.state.transition(NetworkState::Ready)?;

        kmsg::info!("Network initialization complete");

        Ok(())
    }

    async fn reset(&mut self) {
        let handles: Vec<_> = self.interfaces.drain().map(|(_, handle)| handle).collect();
        for handle in handles {
            drop(handle.command_tx.send(Command::Shutdown).await);
        }
        self.owners.clear();
        self.state = NetworkSnapshot::empty();
    }

    async fn handle_command(&mut self, cmd: SupervisorCommand) {
        match cmd {
            SupervisorCommand::Initialize { reply } => {
                let result = self.initialize().await;
                drop(reply.send(result));
            }
            SupervisorCommand::Reconcile { reply } => {
                reconcile::run(self).await;
                let _sent = reply.send(());
            }
            SupervisorCommand::Status { reply } => {
                drop(reply.send(self.status()));
            }
        }
    }

    fn get_primary_name(&self) -> Result<Name> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    fn status(&self) -> Status {
        let owner = owner::l3_owner(&self.state, &self.interfaces, &self.owners);
        let owner_link = owner
            .and_then(|owner| self.interfaces.get(owner))
            .map(|handle| handle.snapshot_rx.borrow().link);
        Status {
            state: self.state.state,
            primary: self.state.primary.clone(),
            owner: owner.cloned(),
            owner_link,
            backups: self.state.backups.clone(),
            processed_events: self.processed_events,
        }
    }

    fn spawn_interface(&mut self, link: Ethernet) {
        let name = link.name.clone();
        let handle = Interface::spawn(self.ops.clone(), link);
        self.owners
            .entry(name.clone())
            .or_insert_with(|| name.clone());
        self.interfaces.insert(name, handle);
    }

    async fn send_to_interface(&self, name: &Name, cmd: Command) {
        if let Some(handle) = self.interfaces.get(name) {
            drop(handle.command_tx.send(cmd).await);
        }
    }
}

impl NetworkActorHandle {
    /// Triggers a single reconciliation pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervisor command channel is closed.
    pub async fn reconcile(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(SupervisorCommand::Reconcile { reply }).await?;
        rx.await?;

        Ok(())
    }

    /// Returns the supervisor's current view of the network.
    ///
    /// # Errors
    ///
    /// Returns an error if the supervisor command channel is closed.
    pub async fn status(&self) -> Result<Status> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(SupervisorCommand::Status { reply }).await?;
        Ok(rx.await?)
    }

    /// Initializes the network, retrying with exponential backoff until success.
    ///
    /// # Errors
    ///
    /// Returns an error when the supervisor command channel is closed.
    pub async fn initialize_with_retry(&self) -> Result<()> {
        let base_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(10);

        retry::initialize(self, base_delay, max_delay).await
    }

    async fn initialize(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Initialize { reply })
            .await?;
        rx.await??;

        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Subscribes to network link events.
pub trait Source {
    /// Opens a new subscription, retrying until it succeeds.
    fn subscribe(&self) -> impl Future<Output = Result<mpsc::Receiver<Event>>> + Send;
}

/// Starts the network supervisor with a live `netlink` connection.
///
/// Must be called from within a Tokio runtime.
///
/// # Errors
///
/// Returns an error if the `netlink` connection cannot be established, the
/// network event monitor cannot be started, or the network configuration
/// cannot be loaded.
pub async fn start() -> Result<NetworkActorHandle> {
    let ops = Rtnl::connect()?;
    let event_rx = monitor::start().await?;
    let config = Arc::new(network()?.clone());

    Ok(start_with(ops, event_rx, config, Resolver::default()))
}

/// Starts the supervisor with injected dependencies.
pub fn start_with<N: Ops>(
    ops: N,
    event_rx: mpsc::Receiver<Event>,
    config: Arc<network::Config>,
    dns: Resolver,
) -> NetworkActorHandle {
    start_with_source(ops, event_rx, monitor::Live, config, dns)
}

/// Starts the supervisor with injected dependencies and event source.
pub fn start_with_source<N: Ops, M: Source + Send + Sync + 'static>(
    ops: N,
    event_rx: mpsc::Receiver<Event>,
    source: M,
    config: Arc<network::Config>,
    dns: Resolver,
) -> NetworkActorHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);

    tokio::spawn(supervisor_loop(ops, cmd_rx, event_rx, source, config, dns));

    NetworkActorHandle { tx: cmd_tx }
}

async fn supervisor_loop<N: Ops, M: Source + Send + Sync + 'static>(
    ops: N,
    mut cmd_rx: mpsc::Receiver<SupervisorCommand>,
    event_rx: mpsc::Receiver<Event>,
    source: M,
    config: Arc<network::Config>,
    dns: Resolver,
) {
    let mut supervisor = NetworkSupervisor::new(ops, config, dns);
    let mut events = monitor::Subscription::new(event_rx, source);
    let mut reconcile = interval(RECONCILE_INTERVAL);
    // Startup provisioning already applies intent, so skip the immediate first tick.
    reconcile.tick().await;

    loop {
        let owner = owner::l3_owner(
            &supervisor.state,
            &supervisor.interfaces,
            &supervisor.owners,
        )
        .cloned();
        let owner_snap_rx = owner
            .as_ref()
            .and_then(|owner| supervisor.interfaces.get_mut(owner))
            .map(|handle| &mut handle.snapshot_rx);

        let event = selection::wait(
            &mut cmd_rx,
            events.receiver(),
            owner_snap_rx,
            &mut reconcile,
        )
        .await;
        let Some(event) = event else {
            kmsg::info!("Network supervisor shutting down");
            break;
        };

        match event {
            SupervisorEvent::Command(cmd) => supervisor.handle_command(cmd).await,
            SupervisorEvent::Netlink(event) => {
                events.handle(&mut supervisor, event).await;
                supervisor.processed_events = supervisor.processed_events.saturating_add(1);
            }
            SupervisorEvent::OwnerChanged => {
                failover::switch_if_needed(&mut supervisor).await;
                failover::recover_if_possible(&mut supervisor);
                dns::flush(&mut supervisor);
            }
            SupervisorEvent::ReconcileTick => {
                reconcile::run(&mut supervisor).await;
                events.retry_if_closed().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snap() -> NetworkSnapshot {
        NetworkSnapshot::empty()
    }

    #[test]
    fn network_state_display() {
        // ACT / ASSERT
        assert_eq!(NetworkState::Uninitialized.to_string(), "Uninitialized");
        assert_eq!(NetworkState::Initializing.to_string(), "Initializing");
        assert_eq!(NetworkState::Operational.to_string(), "Operational");
        assert_eq!(NetworkState::Ready.to_string(), "Ready");
        assert_eq!(NetworkState::Degraded.to_string(), "Degraded");
    }

    #[test]
    fn valid_transitions_succeed() {
        let pairs = [
            (NetworkState::Uninitialized, NetworkState::Initializing),
            (NetworkState::Initializing, NetworkState::Operational),
            (NetworkState::Initializing, NetworkState::Degraded),
            (NetworkState::Operational, NetworkState::Ready),
            (NetworkState::Operational, NetworkState::Degraded),
            (NetworkState::Ready, NetworkState::Degraded),
            (NetworkState::Degraded, NetworkState::Initializing),
            (NetworkState::Degraded, NetworkState::Operational),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = empty_snap();
            snap.state = from;

            // ACT
            let result = snap.transition(to);

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(snap.state, to);
        }
    }

    #[test]
    fn invalid_transitions_return_error() {
        let pairs = [
            (NetworkState::Uninitialized, NetworkState::Operational),
            (NetworkState::Uninitialized, NetworkState::Ready),
            (NetworkState::Uninitialized, NetworkState::Degraded),
            (NetworkState::Initializing, NetworkState::Ready),
            (NetworkState::Initializing, NetworkState::Uninitialized),
            (NetworkState::Operational, NetworkState::Uninitialized),
            (NetworkState::Operational, NetworkState::Initializing),
            (NetworkState::Ready, NetworkState::Operational),
            (NetworkState::Ready, NetworkState::Uninitialized),
            (NetworkState::Ready, NetworkState::Initializing),
            (NetworkState::Degraded, NetworkState::Ready),
            (NetworkState::Degraded, NetworkState::Uninitialized),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = empty_snap();
            snap.state = from;

            // ACT
            let result = snap.transition(to);

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(
                snap.state, from,
                "state must not change on invalid transition"
            );
        }
    }

    #[test]
    fn transition_does_not_mutate_state_on_error() {
        // ARRANGE
        let mut snap = empty_snap();
        snap.state = NetworkState::Ready;

        // ACT
        let _result = snap.transition(NetworkState::Initializing);

        // ASSERT
        assert_eq!(snap.state, NetworkState::Ready);
    }
}
