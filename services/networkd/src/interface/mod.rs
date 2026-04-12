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
use std::pin::Pin;

use anyhow::Result;
pub use commands::InterfaceCommand;
use dns::DnsState;
use rtnetlink::Handle;
use snapshot::InterfaceSnapshot;
use state::InterfaceState;
use tokio::sync::{mpsc, watch};
use tokio::time::Sleep;

use crate::dhcp::DhcpLease;
use crate::slaac::{SlaacEvent, SlaacManager};

pub struct InterfaceActor {
    snapshot: InterfaceSnapshot,
    handle: Handle,
    cmd_rx: mpsc::Receiver<InterfaceCommand>,
    snapshot_tx: watch::Sender<InterfaceSnapshot>,
    dns: DnsState,
    renew_at: Option<Pin<Box<Sleep>>>,
    rebind_at: Option<Pin<Box<Sleep>>>,
    expire_at: Option<Pin<Box<Sleep>>>,
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
            renew_at: None,
            rebind_at: None,
            expire_at: None,
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
                _ = poll_opt(&mut self.renew_at) => {
                    self.renew_lease().await;
                }
                _ = poll_opt(&mut self.rebind_at) => {
                    self.rebind_lease().await;
                }
                _ = poll_opt(&mut self.expire_at) => {
                    if let Err(e) = self.do_full_dora().await {
                        kmsg::warn!("DHCP re-acquire failed for {}: {}", self.snapshot.name, e);
                    }
                }
                event = slaac_next_event(&mut self.slaac) => {
                    self.handle_slaac_event(event).await;
                }
            }
        }

        kmsg::info!("InterfaceActor stopped for {}", self.snapshot.name);
    }

    async fn dispatch(&mut self, cmd: InterfaceCommand) {
        match cmd {
            InterfaceCommand::ConfigureDhcp => {
                self.run_dhcp().await;
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
            InterfaceCommand::ConfigureSlaac => self.configure_slaac().await,
            InterfaceCommand::LinkUp => self.on_link_up().await,
            InterfaceCommand::LinkDown => self.on_link_down(),
            InterfaceCommand::Shutdown => {
                kmsg::info!("InterfaceActor shutting down for {}", self.snapshot.name);
                self.cmd_rx.close();
            }
        }
    }

    async fn configure_slaac(&mut self) {
        let iface = self.snapshot.name.to_string();
        let mac = self.snapshot.mac;

        kmsg::info!("Starting SLAAC on {}", iface);

        match SlaacManager::new(iface.clone(), mac) {
            Ok(mut mgr) => match mgr.solicit().await {
                Ok(event) => {
                    self.handle_slaac_event(event).await;
                    self.slaac = Some(mgr);
                    kmsg::info!("SLAAC monitoring active on {}", iface);
                }
                Err(e) => {
                    kmsg::warn!(
                        "SLAAC solicitation failed on {}: {} (continuing with IPv4)",
                        iface,
                        e
                    );
                }
            },
            Err(e) => {
                kmsg::info!(
                    "SLAAC unavailable on {}: {} (continuing with IPv4)",
                    iface,
                    e
                );
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

    async fn on_link_up(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Up;
        if self.snapshot.state != InterfaceState::Degraded {
            return;
        }
        if let Some(lease) = self.snapshot.lease.clone() {
            self.recover_with_lease(lease).await;
        } else if let Err(e) = self.do_full_dora().await {
            kmsg::warn!(
                "DHCP re-acquire failed on link-up for {}: {}",
                self.snapshot.name,
                e
            );
        }
        self.publish_snapshot();
    }

    async fn recover_with_lease(&mut self, lease: DhcpLease) {
        if let Err(e) = self.apply_lease(self.snapshot.index, &lease).await {
            kmsg::warn!(
                "Failed to re-apply lease on link-up for {}: {}",
                self.snapshot.name,
                e
            );
            self.set_state(InterfaceState::Failed);
            return;
        }
        self.arm_lease_timers(&lease);
        if let Err(e) = self.snapshot.transition(InterfaceState::Configured) {
            kmsg::warn!(
                "Interface {} state transition failed on link-up: {}",
                self.snapshot.name,
                e
            );
        }
    }

    fn on_link_down(&mut self) {
        self.snapshot.link = netlib::link::LinkStateKind::Down;
        self.disarm_lease_timers();
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
