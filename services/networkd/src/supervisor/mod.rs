//! Network supervisor that routes events, manages per-interface actors, and aggregates state.

mod commands;
mod discovery;
mod dispatch;
mod failover;
mod provision;
mod snapshot;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use commands::SupervisorCommand;
use netlib::interface::InterfaceName;
use netlib::monitor::{self, NetworkEvent};
use netlib::ops::{NetlinkOps, RtnetlinkOps};
use tokio::sync::{mpsc, oneshot, watch};

use crate::dns::DnsState;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::{InterfaceActor, InterfaceActorHandle, InterfaceCommand};
use crate::supervisor::snapshot::NetworkSnapshot;
use crate::supervisor::state::NetworkState;

struct NetworkSupervisor<N: NetlinkOps> {
    ops: N,
    config: Arc<config::NetworkConfig>,
    state: NetworkSnapshot,
    interfaces: HashMap<InterfaceName, InterfaceActorHandle>,
    watch_tx: watch::Sender<NetworkSnapshot>,
    dns: DnsState,
}

impl<N: NetlinkOps> NetworkSupervisor<N> {
    fn new(
        ops: N,
        config: Arc<config::NetworkConfig>,
        watch_tx: watch::Sender<NetworkSnapshot>,
        dns: DnsState,
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

    /// Publishes the current aggregated state to subscribers.
    fn publish_state(&self) {
        let _ = self.watch_tx.send(self.state.clone());
    }

    /// Synchronizes the aggregated state from all interfaces and publishes it.
    fn sync_and_publish(&mut self) {
        self.state.interfaces = self
            .interfaces
            .values()
            .map(|h| Arc::new(h.state_rx.borrow().clone()))
            .collect();
        self.flush_dns();
        self.publish_state();
    }

    /// Flushes DNS configuration based on the primary interface's state with fallback.
    fn flush_dns(&mut self) {
        let Some(primary) = &self.state.primary else {
            return;
        };
        let Some(handle) = self.interfaces.get(primary) else {
            return;
        };
        let snap = handle.state_rx.borrow();

        let v4 = snap
            .ip
            .as_ref()
            .filter(|c| !c.dns.is_empty())
            .map(|c| c.dns.clone())
            .unwrap_or_else(|| self.config.ipv4_dns());

        let v6 = snap
            .ipv6
            .as_ref()
            .filter(|c| !c.dns.is_empty())
            .map(|c| c.dns.clone())
            .unwrap_or_else(|| self.config.ipv6_dns());

        if self.dns.is_unchanged(&v4, &v6) {
            return;
        }

        if let Err(e) = self.dns.update(v4, v6) {
            kmsg::warn!("Failed to write DNS: {}", e);
        }
    }

    fn get_primary_name(&self) -> Result<InterfaceName> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    fn spawn_interface_actor(&mut self, snapshot: InterfaceSnapshot) {
        let name = snapshot.name.clone();
        let actor_handle =
            InterfaceActor::spawn(snapshot, self.ops.clone(), Arc::clone(&self.config));
        self.interfaces.insert(name, actor_handle);
    }

    async fn send_to_interface(&self, name: &InterfaceName, cmd: InterfaceCommand) {
        if let Some(handle) = self.interfaces.get(name) {
            let _ = handle.cmd_tx.send(cmd).await;
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        kmsg::info!("Initializing network");

        self.reset().await;
        self.discover_interfaces().await?;
        self.provision_interfaces().await;

        self.state.transition(NetworkState::Ready)?;
        self.publish_state();

        kmsg::info!("Network initialization complete");

        Ok(())
    }

    async fn reset(&mut self) {
        for (_, handle) in self.interfaces.drain() {
            let _ = handle.cmd_tx.send(InterfaceCommand::Shutdown).await;
        }
        self.state = NetworkSnapshot::empty();
    }

    async fn handle_command(&mut self, cmd: SupervisorCommand) {
        match cmd {
            SupervisorCommand::Initialize { reply } => {
                let result = self.initialize().await;
                let _ = reply.send(result);
            }
        }
    }
}

#[derive(Clone)]
pub struct NetworkActorHandle {
    tx: mpsc::Sender<SupervisorCommand>,
}

impl NetworkActorHandle {
    async fn initialize(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(SupervisorCommand::Initialize { reply })
            .await?;
        rx.await??;
        Ok(())
    }

    pub async fn initialize_with_retry(&self) -> Result<()> {
        let base_delay = std::time::Duration::from_secs(1);
        let max_delay = std::time::Duration::from_secs(10);

        for attempt in 1u32.. {
            match self.try_initialize(attempt, base_delay, max_delay).await {
                Some(ok) => return ok,
                None => continue,
            }
        }

        unreachable!()
    }

    async fn try_initialize(
        &self,
        attempt: u32,
        base_delay: std::time::Duration,
        max_delay: std::time::Duration,
    ) -> Option<Result<()>> {
        if self.initialize().await.is_ok() {
            println!("Network initialized successfully on attempt {}", attempt);
            return Some(Ok(()));
        }
        eprintln!("Network initialization failed (attempt {})", attempt);

        let delay = retry_delay(base_delay, max_delay, attempt);
        println!("Retrying in {:?}...", delay);
        tokio::time::sleep(delay).await;
        None
    }
}

fn retry_delay(
    base: std::time::Duration,
    max: std::time::Duration,
    attempt: u32,
) -> std::time::Duration {
    let multiplier = 1u32 << attempt.saturating_sub(1).min(5);
    base.checked_mul(multiplier).unwrap_or(max).min(max)
}

/// Starts the network supervisor with the rtnetlink backend.
pub async fn start() -> Result<NetworkActorHandle> {
    let (connection, handle, _) = rtnetlink::new_connection()?;
    tokio::spawn(connection);

    let ops = RtnetlinkOps::new(handle.clone());
    let event_rx = start_events_monitor().await;
    let config = Arc::new(config::network().clone());

    start_with(ops, event_rx, config, DnsState::default())
}

/// Starts the supervisor with injected dependencies.
pub fn start_with<N: NetlinkOps>(
    ops: N,
    event_rx: Option<mpsc::Receiver<NetworkEvent>>,
    config: Arc<config::NetworkConfig>,
    dns: DnsState,
) -> Result<NetworkActorHandle> {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (watch_tx, _) = watch::channel(NetworkSnapshot::empty());

    run(ops, cmd_rx, event_rx, watch_tx, config, dns);

    Ok(NetworkActorHandle { tx: cmd_tx })
}

async fn start_events_monitor() -> Option<mpsc::Receiver<NetworkEvent>> {
    let config = monitor::MonitorConfig::default();
    match monitor::start(config).await {
        Ok(rx) => {
            println!("Network event monitoring enabled");
            Some(rx)
        }
        Err(e) => {
            eprintln!("Failed to start network monitor: {}", e);
            None
        }
    }
}

fn run<N: NetlinkOps>(
    ops: N,
    mut cmd_rx: mpsc::Receiver<SupervisorCommand>,
    mut event_rx: Option<mpsc::Receiver<NetworkEvent>>,
    watch_tx: watch::Sender<NetworkSnapshot>,
    config: Arc<config::NetworkConfig>,
    dns: DnsState,
) {
    tokio::spawn(async move {
        let mut supervisor = NetworkSupervisor::new(ops, config, watch_tx, dns);

        loop {
            let mut primary_snap_rx = supervisor
                .state
                .primary
                .as_ref()
                .and_then(|p| supervisor.interfaces.get(p))
                .map(|h| h.state_rx.clone());

            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    supervisor.handle_command(cmd).await;
                }

                Some(event) = async {
                    match &mut event_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    supervisor.handle_event(event).await;
                }

                Ok(()) = async {
                    match &mut primary_snap_rx {
                        Some(rx) => rx.changed().await,
                        None => std::future::pending().await,
                    }
                } => {
                    supervisor.flush_dns();
                }

                else => {
                    println!("Network supervisor shutting down");
                    break;
                }
            }
        }
    });
}
