//! Per-interface actor that owns one interface's life cycle, DHCP, SLAAC, and static IP.

mod bridge;
mod commands;
mod dhcp;
mod dns;
mod slaac;
pub mod snapshot;
pub mod state;
mod r#static;

use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::Result;
pub use commands::InterfaceCommand;
use dns::DnsState;
use rtnetlink::Handle;
use snapshot::InterfaceSnapshot;
use state::InterfaceState;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

pub struct InterfaceActor {
    snapshot: InterfaceSnapshot,
    handle: Handle,
    cmd_rx: mpsc::Receiver<InterfaceCommand>,
    self_tx: mpsc::Sender<InterfaceCommand>,
    snapshot_tx: watch::Sender<InterfaceSnapshot>,
    dns: DnsState,
    renewal_tasks: Vec<JoinHandle<()>>,
    has_ipv6: bool,
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
            self_tx: cmd_tx.clone(),
            snapshot_tx,
            dns: DnsState::default(),
            renewal_tasks: Vec::new(),
            has_ipv6: false,
        };

        tokio::spawn(actor.run());

        InterfaceActorHandle { cmd_tx, state_rx }
    }

    async fn run(mut self) {
        kmsg::info!("InterfaceActor started for {}", self.snapshot.name);

        while let Some(cmd) = self.cmd_rx.recv().await {
            self.dispatch(cmd).await;
        }

        self.cancel_renewal_tasks();
        kmsg::info!("InterfaceActor stopped for {}", self.snapshot.name);
    }

    async fn dispatch(&mut self, cmd: InterfaceCommand) {
        match cmd {
            InterfaceCommand::ConfigureDhcp { reply } => {
                let _ = reply.send(self.acquire_dhcp().await);
            }
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
            InterfaceCommand::LeaseAction(action) => self.handle_lease_action(action).await,
            InterfaceCommand::StartSlaac => self.try_acquire_slaac().await,
            InterfaceCommand::Slaac(event) => self.handle_slaac_event(event).await,
            InterfaceCommand::LinkUp => self.on_link_up(),
            InterfaceCommand::LinkDown => self.on_link_down(),
            InterfaceCommand::Shutdown => {
                kmsg::info!("InterfaceActor shutting down for {}", self.snapshot.name);
                self.cmd_rx.close();
            }
        }
    }

    async fn try_apply_static_ipv4(
        &mut self,
        index: u32,
        addresses: &[config::Cidr4],
        gateway: Option<Ipv4Addr>,
    ) {
        if let Err(e) = self.apply_static_ipv4(index, addresses, gateway).await {
            kmsg::warn!("Static IPv4 failed on {}: {}", self.snapshot.name, e);
        }
    }

    async fn try_apply_static_ipv6(
        &mut self,
        index: u32,
        addresses: &[config::Cidr6],
        gateway: Option<std::net::Ipv6Addr>,
    ) {
        if let Err(e) = self.apply_static_ipv6(index, addresses, gateway).await {
            kmsg::warn!("Static IPv6 failed on {}: {}", self.snapshot.name, e);
        }
    }

    fn on_link_up(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Up;
        if self.snapshot.state != InterfaceState::Degraded {
            return;
        }
        let target = if self.snapshot.lease.is_some() {
            InterfaceState::Configured
        } else {
            InterfaceState::Configuring
        };
        if let Err(e) = self.snapshot.transition(target) {
            kmsg::warn!(
                "Interface {} state transition failed on link-up: {}",
                self.snapshot.name,
                e
            );
        }
        self.publish_snapshot();
    }

    fn on_link_down(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Down;
        if self.snapshot.state == InterfaceState::Configured
            && let Err(e) = self.snapshot.transition(InterfaceState::Degraded)
        {
            kmsg::warn!(
                "Interface {} state transition failed on link-down: {}",
                self.snapshot.name,
                e
            );
        }
        self.publish_snapshot();
    }

    fn set_state(&mut self, state: InterfaceState) {
        if let Err(e) = self.snapshot.transition(state) {
            kmsg::warn!("{}", e);
        }
    }

    fn publish_snapshot(&self) {
        let _ = self.snapshot_tx.send(self.snapshot.clone());
    }

    /// Updates IPv4 DNS servers and flushes resolv.conf.
    fn update_dns_v4(&mut self, servers: Vec<Ipv4Addr>) -> Result<()> {
        self.dns.v4 = servers;
        self.dns.flush()
    }

    /// Updates IPv6 DNS servers and flushes resolv.conf.
    fn update_dns_v6(&mut self, servers: Vec<Ipv6Addr>) -> Result<()> {
        self.dns.v6 = servers;
        self.dns.flush()
    }
}
