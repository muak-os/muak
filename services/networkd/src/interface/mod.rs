//! Per-interface actor that owns one interface's life cycle, DHCP, SLAAC, and static IP.

mod bridge;
pub mod commands;
mod dhcp;
mod link;
mod slaac;
pub mod snapshot;
pub mod state;
mod r#static;

use alloc::sync::Arc;
use core::pin::Pin;

use commands::Command;
use dhcp::LeaseTimers;
use netlib::netlink::Ops;
use snapshot::Snapshot;
use state::Lifecycle;
use tokio::sync::{mpsc, watch};
use tokio::time::Sleep;

use crate::dhcp::Lease;
use crate::dhcp::client::{DhcpConnector, SystemDhcpConnector};
use crate::dhcp::manager::Manager;
use crate::slaac::manager::{Manager as SlaacManager, SlaacEvent};

pub struct Actor<N: Ops> {
    snapshot: Snapshot,
    ops: N,
    config: Arc<config::NetworkConfig>,
    cmd_rx: mpsc::Receiver<Command>,
    snapshot_tx: watch::Sender<Arc<Snapshot>>,
    dhcp: Option<Manager>,
    timers: LeaseTimers,
    slaac: Option<SlaacManager>,
}

/// Handle used by the supervisor to send commands and watch state.
pub struct ActorHandle {
    pub cmd_tx: mpsc::Sender<Command>,
    pub state_rx: watch::Receiver<Arc<Snapshot>>,
}

/// A single event selected from the actor's input sources.
enum ActorEvent {
    Command(Command),
    DhcpLease(Lease),
    Slaac(SlaacEvent),
    Renew,
    Rebind,
    Expire,
}

impl<N: Ops> Actor<N> {
    /// Spawns a new per-interface actor.
    pub fn spawn(snapshot: Snapshot, ops: N, config: Arc<config::NetworkConfig>) -> ActorHandle {
        Self::spawn_with(snapshot, ops, config, SystemDhcpConnector)
    }

    /// Spawns a new per-interface actor with a custom DHCP connector.
    pub fn spawn_with<C: DhcpConnector>(
        snapshot: Snapshot,
        ops: N,
        config: Arc<config::NetworkConfig>,
        connector: C,
    ) -> ActorHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (snapshot_tx, state_rx) = watch::channel(Arc::new(snapshot.clone()));

        let mut actor = Self {
            snapshot,
            ops,
            config,
            cmd_rx,
            snapshot_tx,
            dhcp: None,
            timers: LeaseTimers::new(),
            slaac: None,
        };

        actor.rehydrate_runtime_state();

        tokio::spawn(actor.run(connector));

        ActorHandle { cmd_tx, state_rx }
    }

    /// Rehydrates runtime-only state from a persisted interface snapshot.
    async fn run<C: DhcpConnector>(mut self, connector: C) {
        kmsg::info!("Actor started for {}", self.snapshot.name);
        actor_loop(&mut self, &connector).await;
        kmsg::info!("Actor stopped for {}", self.snapshot.name);
    }

    /// Runs the full DHCP re-acquisition when the lease expires.
    async fn dispatch<C: DhcpConnector>(&mut self, cmd: Command, connector: &C) {
        match cmd {
            Command::ConfigureDhcp { mode } => dhcp::apply(self, mode, connector).await,
            Command::ConfigureStaticIpv4 {
                mode,
                index,
                addresses,
                gateway,
            } => {
                r#static::apply_ipv4(self, index, &addresses, gateway, mode).await;
            }
            Command::ConfigureStaticIpv6 {
                mode,
                index,
                addresses,
                gateway,
            } => {
                r#static::apply_ipv6(self, index, &addresses, gateway, mode).await;
            }
            Command::ConfigureBridge {
                bridge_name,
                stp,
                reply,
            } => {
                drop(reply.send(bridge::configure(self, &bridge_name, stp).await));
            }
            Command::ConfigureSlaac { mode } => slaac::apply(self, mode).await,
            Command::LinkUp => link::up(self, connector).await,
            Command::LinkDown => link::down(self),
            Command::Shutdown => {
                kmsg::info!("Actor shutting down for {}", self.snapshot.name);
                self.dhcp = None;
                self.cmd_rx.close();
            }
        }
    }

    fn rehydrate_runtime_state(&mut self) {
        if let Some(lease) = self.snapshot.lease.as_ref() {
            self.timers.arm(lease);
        }
    }

    async fn on_expire<C: DhcpConnector>(&mut self, connector: &C) {
        if let Err(error) = dhcp::do_full_dora(self, connector).await {
            kmsg::warn!("DHCP re-acquire failed for {}: {error}", self.snapshot.name);
        }
    }

    fn set_state(&mut self, state: Lifecycle) {
        if let Err(e) = self.snapshot.transition(state) {
            kmsg::warn!("{}", e);
            return;
        }
        self.publish_snapshot();
    }

    fn publish_snapshot(&self) {
        drop(self.snapshot_tx.send(Arc::new(self.snapshot.clone())));
    }

    /// Advances the interface to `Configured` when reconciliation succeeds.
    fn ensure_configured_state(&mut self) -> bool {
        match self.snapshot.state {
            Lifecycle::Configured | Lifecycle::Deconfiguring => false,
            Lifecycle::Configuring | Lifecycle::Degraded => {
                self.set_state(Lifecycle::Configured);
                true
            }
            Lifecycle::Discovered | Lifecycle::Failed => {
                self.set_state(Lifecycle::Configuring);
                self.set_state(Lifecycle::Configured);
                true
            }
        }
    }
}

async fn actor_loop<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) {
    loop {
        let event = actor_select(
            &mut actor.cmd_rx,
            &mut actor.dhcp,
            &mut actor.slaac,
            &mut actor.timers.renew,
            &mut actor.timers.rebind,
            &mut actor.timers.expire,
        )
        .await;
        let Some(event) = event else {
            break;
        };

        match event {
            ActorEvent::Command(cmd) => actor.dispatch(cmd, connector).await,
            ActorEvent::DhcpLease(lease) => dhcp::acquired(actor, lease).await,
            ActorEvent::Slaac(event) => slaac::handle_event(actor, event).await,
            ActorEvent::Renew => dhcp::renew_lease(actor, connector).await,
            ActorEvent::Rebind => dhcp::rebind_lease(actor, connector).await,
            ActorEvent::Expire => actor.on_expire(connector).await,
        }
    }
}

/// Waits for the next command, DHCP/SLAAC event, or timer expiration.
async fn actor_select(
    cmd_rx: &mut mpsc::Receiver<Command>,
    dhcp: &mut Option<Manager>,
    slaac: &mut Option<SlaacManager>,
    renew: &mut Option<Pin<Box<Sleep>>>,
    rebind: &mut Option<Pin<Box<Sleep>>>,
    expire: &mut Option<Pin<Box<Sleep>>>,
) -> Option<ActorEvent> {
    let mut cmd_fut = core::pin::pin!(cmd_rx.recv());
    let mut dhcp_fut = core::pin::pin!(dhcp_acquire(dhcp));
    let mut slaac_fut = core::pin::pin!(slaac_next_event(slaac));
    let mut renew_fut = core::pin::pin!(poll_opt(renew));
    let mut rebind_fut = core::pin::pin!(poll_opt(rebind));
    let mut expire_fut = core::pin::pin!(poll_opt(expire));

    core::future::poll_fn(|cx| {
        if let core::task::Poll::Ready(Some(cmd)) = cmd_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(ActorEvent::Command(cmd)));
        }
        if let core::task::Poll::Ready(lease) = dhcp_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(ActorEvent::DhcpLease(lease)));
        }
        if let core::task::Poll::Ready(event) = slaac_fut.as_mut().poll(cx) {
            return core::task::Poll::Ready(Some(ActorEvent::Slaac(event)));
        }
        if renew_fut.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Some(ActorEvent::Renew));
        }
        if rebind_fut.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Some(ActorEvent::Rebind));
        }
        if expire_fut.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Some(ActorEvent::Expire));
        }
        core::task::Poll::Pending
    })
    .await
}

/// Polls a `Sleep` future when `Some`, or returns `core::future::pending()` when `None`.
async fn poll_opt(opt: &mut Option<Pin<Box<Sleep>>>) {
    match opt.as_mut() {
        Some(sleep) => {
            sleep.await;
            *opt = None;
        }
        None => core::future::pending::<()>().await,
    }
}

/// Polls the `SlaacManager`'s next event when `Some`, or parks forever when `None`.
async fn slaac_next_event(slaac: &mut Option<SlaacManager>) -> SlaacEvent {
    match slaac.as_mut() {
        Some(mgr) => mgr.next_event().await,
        None => core::future::pending().await,
    }
}

/// Drives a `Manager` when `Some`, or parks forever when `None`.
async fn dhcp_acquire(dhcp: &mut Option<Manager>) -> Lease {
    match dhcp.as_mut() {
        Some(mgr) => mgr.acquire().await,
        None => core::future::pending().await,
    }
}
