//! Network supervisor that routes events, manages per-interface actors, and aggregates state.

mod commands;
mod discovery;
mod dispatch;
mod dns;
mod failover;
mod monitor;
mod provision;
mod reconcile;
mod retry;
mod selection;
mod snapshot;
mod state;

use alloc::sync::Arc;
use core::time::Duration;
use std::collections::HashMap;

use anyhow::Result;
use commands::SupervisorCommand;
use config::system::network;
use netdev::Ops;
use netlink::Rtnl;
use netlink::link::ifname::Name;
use netlink_monitor::Event;
use selection::SupervisorEvent;
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

use crate::dns::Resolver;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;
use crate::interface::{Actor, ActorHandle};
use crate::supervisor::snapshot::NetworkSnapshot;
use crate::supervisor::state::NetworkState;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

struct NetworkSupervisor<N: Ops> {
    ops: N,
    config: Arc<network::Config>,
    state: NetworkSnapshot,
    interfaces: HashMap<Name, ActorHandle>,
    dns: Resolver,
}

#[derive(Clone)]
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
            dns,
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
            drop(handle.cmd_tx.send(Command::Shutdown).await);
        }
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
        }
    }

    fn get_primary_name(&self) -> Result<Name> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    fn spawn_interface_actor(&mut self, snapshot: Snapshot) {
        let name = snapshot.name.clone();
        let actor_handle = Actor::spawn(snapshot, self.ops.clone(), Arc::clone(&self.config));
        self.interfaces.insert(name, actor_handle);
    }

    async fn send_to_interface(&self, name: &Name, cmd: Command) {
        if let Some(handle) = self.interfaces.get(name) {
            drop(handle.cmd_tx.send(cmd).await);
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
/// Returns an error if the `netlink` connection cannot be established or the
/// network event monitor cannot be started.
pub async fn start() -> Result<NetworkActorHandle> {
    let ops = Rtnl::connect()?;
    let event_rx = monitor::start().await?;
    let config = Arc::new(network()?.clone());

    start_with(ops, event_rx, config, Resolver::default())
}

/// Starts the supervisor with injected dependencies.
///
/// # Errors
///
/// Returns an error if the supervisor command channel cannot be created.
pub fn start_with<N: Ops>(
    ops: N,
    event_rx: mpsc::Receiver<Event>,
    config: Arc<network::Config>,
    dns: Resolver,
) -> Result<NetworkActorHandle> {
    start_with_source(ops, event_rx, monitor::Live, config, dns)
}

/// Starts the supervisor with injected dependencies and event source.
///
/// # Errors
///
/// Returns an error if the supervisor command channel cannot be created.
pub fn start_with_source<N: Ops, M: Source + Send + Sync + 'static>(
    ops: N,
    event_rx: mpsc::Receiver<Event>,
    source: M,
    config: Arc<network::Config>,
    dns: Resolver,
) -> Result<NetworkActorHandle> {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);

    run(ops, cmd_rx, event_rx, source, config, dns);

    Ok(NetworkActorHandle { tx: cmd_tx })
}

fn run<N: Ops, M: Source + Send + Sync + 'static>(
    ops: N,
    cmd_rx: mpsc::Receiver<SupervisorCommand>,
    event_rx: mpsc::Receiver<Event>,
    source: M,
    config: Arc<network::Config>,
    dns: Resolver,
) {
    tokio::spawn(supervisor_loop(ops, cmd_rx, event_rx, source, config, dns));
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

    loop {
        let primary_snap_rx = supervisor
            .state
            .primary
            .as_ref()
            .and_then(|primary| supervisor.interfaces.get_mut(primary))
            .map(|handle| &mut handle.state_rx);

        let event = selection::wait(
            &mut cmd_rx,
            events.receiver(),
            primary_snap_rx,
            &mut reconcile,
        )
        .await;
        let Some(event) = event else {
            kmsg::info!("Network supervisor shutting down");
            break;
        };

        match event {
            SupervisorEvent::Command(cmd) => supervisor.handle_command(cmd).await,
            SupervisorEvent::Netlink(event) => events.handle(&mut supervisor, event).await,
            SupervisorEvent::PrimaryChanged => {
                failover::switch_if_needed(&mut supervisor).await;
                dns::flush(&mut supervisor);
            }
            SupervisorEvent::ReconcileTick => {
                reconcile::run(&mut supervisor).await;
                events.retry_if_closed().await;
            }
        }
    }
}
