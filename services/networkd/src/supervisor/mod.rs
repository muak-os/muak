//! Network supervisor that routes events, manages per-interface actors, and aggregates state.

mod commands;
mod discovery;
mod dispatch;
mod failover;
mod provision;
mod reconcile;
mod snapshot;
mod state;

use alloc::sync::Arc;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::collections::HashMap;

use anyhow::Result;
use commands::SupervisorCommand;
use netlib::interface::Name;
use netlib::monitor::{self, Event};
use netlib::netlink::{Ops, Rtnl};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Interval, interval, sleep};

use crate::dns::Resolver;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;
use crate::interface::{Actor, ActorHandle};
use crate::supervisor::snapshot::NetworkSnapshot;
use crate::supervisor::state::NetworkState;

struct NetworkSupervisor<N: Ops> {
    ops: N,
    config: Arc<config::NetworkConfig>,
    state: NetworkSnapshot,
    interfaces: HashMap<Name, ActorHandle>,
    watch_tx: watch::Sender<NetworkSnapshot>,
    dns: Resolver,
}

#[derive(Clone)]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<SupervisorCommand>,
}

/// A single event selected from the supervisor's input sources.
enum SupervisorEvent {
    Command(SupervisorCommand),
    Netlink(Event),
    PrimaryChanged,
    ReconcileTick,
}

const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

impl<N: Ops> NetworkSupervisor<N> {
    fn new(
        ops: N,
        config: Arc<config::NetworkConfig>,
        watch_tx: watch::Sender<NetworkSnapshot>,
        dns: Resolver,
    ) -> Self {
        Self {
            ops,
            config,
            state: NetworkSnapshot::empty(),
            interfaces: HashMap::new(),
            watch_tx,
            dns,
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        kmsg::info!("Initializing network");

        self.reset().await;
        discovery::interfaces(self).await?;
        provision::interfaces(self).await;

        self.state.transition(NetworkState::Ready)?;
        self.publish_state();

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

    /// Synchronizes the aggregated state from all interfaces and publishes it.
    fn sync_and_publish(&mut self) {
        self.state.interfaces.clear();
        self.state.interfaces.extend(
            self.interfaces
                .values()
                .map(|handle| Arc::clone(&handle.state_rx.borrow())),
        );
        self.flush_dns();
        self.publish_state();
    }

    /// Publishes the current aggregated state to subscribers.
    fn publish_state(&self) {
        drop(self.watch_tx.send(self.state.clone()));
    }

    /// Flushes DNS configuration based on the primary interface's state with fallback.
    fn flush_dns(&mut self) {
        let Some(primary) = self.state.primary.as_ref() else {
            return;
        };
        let Some(handle) = self.interfaces.get(primary) else {
            return;
        };
        let (v4, v6) = self.collect_dns(handle);
        if self.dns.is_unchanged(&v4, &v6) {
            return;
        }
        self.apply_dns(v4, v6);
    }

    fn collect_dns(&self, handle: &ActorHandle) -> (Vec<Ipv4Addr>, Vec<Ipv6Addr>) {
        let snap = handle.state_rx.borrow();
        let ipv4_dns = snap
            .ip
            .as_ref()
            .filter(|ip| !ip.dns.is_empty())
            .map_or_else(|| self.config.ipv4_dns().collect(), |ip| ip.dns.clone());
        let ipv6_dns = snap
            .ipv6
            .as_ref()
            .filter(|ip| !ip.dns.is_empty())
            .map_or_else(|| self.config.ipv6_dns().collect(), |ip| ip.dns.clone());

        (ipv4_dns, ipv6_dns)
    }

    fn apply_dns(&mut self, ipv4_dns: Vec<Ipv4Addr>, ipv6_dns: Vec<Ipv6Addr>) {
        if let Err(e) = self.dns.update(ipv4_dns, ipv6_dns) {
            kmsg::warn!("Failed to write DNS: {e}");
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
    /// Returns the underlying initialization error once retries are exhausted.
    pub async fn initialize_with_retry(&self) -> Result<()> {
        let base_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(10);

        initialize_with_retry(self, base_delay, max_delay).await
    }

    async fn initialize(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Initialize { reply })
            .await?;
        rx.await??;

        Ok(())
    }

    async fn try_initialize(
        &self,
        attempt: u32,
        base_delay: Duration,
        max_delay: Duration,
    ) -> Option<Result<()>> {
        if self.initialize().await.is_ok() {
            println!("Network initialized successfully on attempt {attempt}");
            return Some(Ok(()));
        }
        eprintln!("Network initialization failed (attempt {attempt})");

        let delay = retry_delay(base_delay, max_delay, attempt);
        println!("Retrying in {delay:?}...");
        sleep(delay).await;

        None
    }
}

/// Starts the network supervisor with the rtnetlink backend.
///
/// # Errors
///
/// Returns an error if the rtnetlink connection cannot be established.
pub fn start() -> Result<NetworkActorHandle> {
    let (connection, handle, _) = rtnetlink::new_connection()?;
    tokio::spawn(connection);

    let ops = Rtnl::new(handle.clone());
    let event_rx = start_events_monitor();
    let config = Arc::new(config::network().clone());

    start_with(ops, event_rx, config, Resolver::default())
}

/// Starts the supervisor with injected dependencies.
///
/// # Errors
///
/// Returns an error if the supervisor command channel cannot be created.
pub fn start_with<N: Ops>(
    ops: N,
    event_rx: Option<mpsc::Receiver<Event>>,
    config: Arc<config::NetworkConfig>,
    dns: Resolver,
) -> Result<NetworkActorHandle> {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (watch_tx, _) = watch::channel(NetworkSnapshot::empty());

    run(ops, cmd_rx, event_rx, watch_tx, config, dns);

    Ok(NetworkActorHandle { tx: cmd_tx })
}

fn run<N: Ops>(
    ops: N,
    cmd_rx: mpsc::Receiver<SupervisorCommand>,
    event_rx: Option<mpsc::Receiver<Event>>,
    watch_tx: watch::Sender<NetworkSnapshot>,
    config: Arc<config::NetworkConfig>,
    dns: Resolver,
) {
    tokio::spawn(supervisor_loop(
        ops, cmd_rx, event_rx, watch_tx, config, dns,
    ));
}

async fn supervisor_loop<N: Ops>(
    ops: N,
    mut cmd_rx: mpsc::Receiver<SupervisorCommand>,
    mut event_rx: Option<mpsc::Receiver<Event>>,
    watch_tx: watch::Sender<NetworkSnapshot>,
    config: Arc<config::NetworkConfig>,
    dns: Resolver,
) {
    let mut supervisor = NetworkSupervisor::new(ops, config, watch_tx, dns);
    let mut reconcile = interval(RECONCILE_INTERVAL);

    loop {
        let mut primary_snap_rx = supervisor
            .state
            .primary
            .as_ref()
            .and_then(|primary| supervisor.interfaces.get(primary))
            .map(|handle| handle.state_rx.clone());

        let event = supervisor_select(
            &mut cmd_rx,
            &mut event_rx,
            &mut primary_snap_rx,
            &mut reconcile,
        )
        .await;
        let Some(event) = event else {
            println!("Network supervisor shutting down");
            break;
        };

        match event {
            SupervisorEvent::Command(cmd) => supervisor.handle_command(cmd).await,
            SupervisorEvent::Netlink(event) => dispatch::handle_event(&mut supervisor, event).await,
            SupervisorEvent::PrimaryChanged => supervisor.flush_dns(),
            SupervisorEvent::ReconcileTick => reconcile::run(&mut supervisor).await,
        }
    }
}

/// Waits for the next command, netlink event, primary snapshot change, or reconcile tick.
async fn supervisor_select(
    cmd_rx: &mut mpsc::Receiver<SupervisorCommand>,
    event_rx: &mut Option<mpsc::Receiver<Event>>,
    primary_snap_rx: &mut Option<watch::Receiver<Arc<Snapshot>>>,

    reconcile: &mut Interval,
) -> Option<SupervisorEvent> {
    let mut cmd_fut = core::pin::pin!(cmd_rx.recv());
    let mut event_fut = core::pin::pin!(async {
        match event_rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => core::future::pending().await,
        }
    });
    let mut snap_fut = core::pin::pin!(async {
        match primary_snap_rx.as_mut() {
            Some(rx) => rx.changed().await,
            None => core::future::pending().await,
        }
    });
    let mut tick_fut = core::pin::pin!(reconcile.tick());

    core::future::poll_fn(|cx| {
        if let core::task::Poll::Ready(Some(cmd)) = cmd_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::Command(cmd)));
        }
        if let core::task::Poll::Ready(Some(event)) = event_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::Netlink(event)));
        }
        if let core::task::Poll::Ready(Ok(())) = snap_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(SupervisorEvent::PrimaryChanged));
        }
        if tick_fut.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Some(SupervisorEvent::ReconcileTick));
        }
        core::task::Poll::Pending
    })
    .await
}

async fn initialize_with_retry(
    handle: &NetworkActorHandle,
    base_delay: Duration,
    max_delay: Duration,
) -> Result<()> {
    let mut attempt = 1_u32;
    loop {
        if let Some(result) = handle.try_initialize(attempt, base_delay, max_delay).await {
            return result;
        }
        attempt = attempt.saturating_add(1);
    }
}

fn retry_delay(base: Duration, max: Duration, attempt: u32) -> Duration {
    let multiplier = 1_u32 << attempt.saturating_sub(1).min(5);

    base.checked_mul(multiplier).unwrap_or(max).min(max)
}

fn start_events_monitor() -> Option<mpsc::Receiver<Event>> {
    let config = monitor::Config::default();
    match monitor::start(config) {
        Ok(rx) => {
            println!("Network event monitoring enabled");
            Some(rx)
        }
        Err(e) => {
            eprintln!("Failed to start network monitor: {e}");
            None
        }
    }
}
