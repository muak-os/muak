//! Per-interface actor that owns one interface's life cycle, DHCP, SLAAC, and static IP.

mod bridge;
mod commands;
mod dhcp;
mod dns;
mod link;
mod slaac;
pub mod snapshot;
pub mod state;
mod r#static;

use std::pin::Pin;

pub use commands::InterfaceCommand;
use dhcp::LeaseTimers;
use dns::DnsState;
use rtnetlink::Handle;
use snapshot::InterfaceSnapshot;
use state::InterfaceState;
use tokio::sync::{mpsc, watch};
use tokio::time::Sleep;

use crate::dhcp::{DhcpLease, DhcpManager};
use crate::slaac::{SlaacEvent, SlaacManager};

pub struct InterfaceActor {
    snapshot: InterfaceSnapshot,
    handle: Handle,
    cmd_rx: mpsc::Receiver<InterfaceCommand>,
    snapshot_tx: watch::Sender<InterfaceSnapshot>,
    dns: DnsState,
    dhcp: Option<DhcpManager>,
    timers: LeaseTimers,
    slaac: Option<SlaacManager>,
}

/// Handle used by the supervisor to send commands and watch state.
pub struct InterfaceActorHandle {
    pub cmd_tx: mpsc::Sender<InterfaceCommand>,
    pub state_rx: watch::Receiver<InterfaceSnapshot>,
}

impl InterfaceActor {
    /// Spawns a new per-interface actor, returning the handle for the supervisor.
    pub fn spawn(snapshot: InterfaceSnapshot, handle: Handle) -> InterfaceActorHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (snapshot_tx, state_rx) = watch::channel(snapshot.clone());

        let actor = Self {
            snapshot,
            handle,
            cmd_rx,
            snapshot_tx,
            dns: DnsState::default(),
            dhcp: None,
            timers: LeaseTimers::new(),
            slaac: None,
        };

        tokio::spawn(actor.run());

        InterfaceActorHandle { cmd_tx, state_rx }
    }

    async fn run(mut self) {
        kmsg::info!("InterfaceActor started for {}", self.snapshot.name);

        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    self.dispatch(cmd).await;
                }
                lease = dhcp_acquire(&mut self.dhcp) => {
                    self.on_dhcp_acquired(lease).await;
                }
                event = slaac_next_event(&mut self.slaac) => {
                    self.handle_slaac_event(event).await;
                }
                _ = poll_opt(&mut self.timers.renew) => {
                    self.renew_lease().await;
                }
                _ = poll_opt(&mut self.timers.rebind) => {
                    self.rebind_lease().await;
                }
                _ = poll_opt(&mut self.timers.expire) => {
                    if let Err(e) = self.do_full_dora().await {
                        kmsg::warn!("DHCP re-acquire failed for {}: {}", self.snapshot.name, e);
                    }
                }
            }
        }

        kmsg::info!("InterfaceActor stopped for {}", self.snapshot.name);
    }

    async fn dispatch(&mut self, cmd: InterfaceCommand) {
        match cmd {
            InterfaceCommand::ConfigureDhcp => self.start_dhcp(),
            InterfaceCommand::ConfigureStaticIpv4 {
                index,
                addresses,
                gateway,
            } => self.try_apply_static_ipv4(index, &addresses, gateway).await,
            InterfaceCommand::ConfigureStaticIpv6 {
                index,
                addresses,
                gateway,
            } => self.try_apply_static_ipv6(index, &addresses, gateway).await,
            InterfaceCommand::ConfigureBridge {
                bridge_name,
                stp,
                reply,
            } => {
                let _ = reply.send(self.configure_bridge(&bridge_name, stp).await);
            }
            InterfaceCommand::ConfigureSlaac => self.start_slaac(),
            InterfaceCommand::LinkUp => self.on_link_up().await,
            InterfaceCommand::LinkDown => self.on_link_down(),
            InterfaceCommand::Shutdown => {
                kmsg::info!("InterfaceActor shutting down for {}", self.snapshot.name);
                self.dhcp = None;
                self.cmd_rx.close();
            }
        }
    }

    fn set_state(&mut self, state: InterfaceState) {
        if let Err(e) = self.snapshot.transition(state) {
            kmsg::warn!("{}", e);
            return;
        }
        self.publish_snapshot();
    }

    fn publish_snapshot(&self) {
        let _ = self.snapshot_tx.send(self.snapshot.clone());
    }
}

/// Polls a `Sleep` future when `Some`, or returns `std::future::pending()` when `None`.
async fn poll_opt(opt: &mut Option<Pin<Box<Sleep>>>) {
    match opt {
        Some(sleep) => {
            sleep.await;
            *opt = None;
        }
        None => std::future::pending::<()>().await,
    }
}

/// Polls the `SlaacManager`'s next event when `Some`, or parks forever when `None`.
async fn slaac_next_event(slaac: &mut Option<SlaacManager>) -> SlaacEvent {
    match slaac {
        Some(mgr) => mgr.next_event().await,
        None => std::future::pending().await,
    }
}

/// Drives a `DhcpManager` when `Some`, or parks forever when `None`.
async fn dhcp_acquire(dhcp: &mut Option<DhcpManager>) -> DhcpLease {
    match dhcp {
        Some(mgr) => mgr.acquire().await,
        None => std::future::pending().await,
    }
}
