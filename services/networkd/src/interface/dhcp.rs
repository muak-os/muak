//! DHCP lease life cycle management for a per-interface actor.

use std::net::Ipv4Addr;
use std::pin::Pin;
use std::time::SystemTime;

use anyhow::Result;
use netlib::address::IpConfig;
use netlib::netlink::Ops;
use tokio::time::Sleep;

use super::InterfaceActor;
use crate::dhcp::codec::DhcpNak;
use crate::dhcp::{self, DhcpConnector, DhcpLease, DhcpManager, DhcpState};
use crate::interface::ApplyMode;
use crate::interface::state::InterfaceState;
use crate::statemachine::StateMachine;

/// Holds the three timers handles that drive the DHCP renewal state machine.
pub(super) struct LeaseTimers {
    pub renew: Option<Pin<Box<Sleep>>>,
    pub rebind: Option<Pin<Box<Sleep>>>,
    pub expire: Option<Pin<Box<Sleep>>>,
}

impl LeaseTimers {
    /// Returns a new instance with all timers disarmed.
    pub fn new() -> Self {
        Self {
            renew: None,
            rebind: None,
            expire: None,
        }
    }

    /// Arms all three timers from the deadlines encoded in `lease`.
    pub fn arm(&mut self, lease: &DhcpLease) {
        self.disarm();
        let now = SystemTime::now();
        self.renew = deadline_to_sleep(now, lease.obtained_at + lease.renewal_time);
        self.rebind = deadline_to_sleep(now, lease.obtained_at + lease.rebind_time);
        self.expire = deadline_to_sleep(now, lease.expiry());
    }

    /// Cancels all active timers.
    pub fn disarm(&mut self) {
        self.renew = None;
        self.rebind = None;
        self.expire = None;
    }
}

impl<N: Ops> InterfaceActor<N> {
    /// Applies DHCP configuration in the selected mode.
    pub(super) async fn apply_dhcp<C: DhcpConnector>(&mut self, mode: ApplyMode, connector: &C) {
        match mode {
            ApplyMode::Provision => self.start_dhcp(connector).await,
            ApplyMode::Reconcile => self.reconcile_dhcp(connector).await,
        }
    }

    /// Initialises a `DhcpManager` (binding the socket) and marks the interface as configuring.
    pub(super) async fn start_dhcp<C: DhcpConnector>(&mut self, connector: &C) {
        self.set_state(InterfaceState::Configuring);
        let mac = self.snapshot.mac;
        match DhcpManager::new(self.snapshot.name.as_str(), mac, connector).await {
            Ok(mgr) => self.dhcp = Some(mgr),
            Err(e) => {
                kmsg::warn!(
                    "Failed to create DHCP socket on {}: {}",
                    self.snapshot.name,
                    e
                );
                self.set_state(InterfaceState::Failed);
            }
        }
    }

    /// Re-applies DHCP state or restarts acquisition when no lease is cached.
    pub(super) async fn reconcile_dhcp<C: DhcpConnector>(&mut self, connector: &C) {
        if let Some(lease) = self.snapshot.lease.clone() {
            self.reconcile_cached_lease(lease).await;
            return;
        }

        if self.dhcp.is_none() {
            self.start_dhcp(connector).await;
        }
    }

    /// Re-applies a cached lease to restore DHCP-managed kernel state.
    async fn reconcile_cached_lease(&mut self, lease: DhcpLease) {
        let index = self.snapshot.index;
        if let Err(e) = self.apply_lease(index, &lease).await {
            kmsg::warn!("DHCP reconcile failed on {}: {}", self.snapshot.name, e);
            return;
        }

        self.store_lease(&lease);
        self.set_dhcp_state(DhcpState::Bound);
        self.timers.arm(&lease);
        let _ = self.ensure_configured_state();
    }

    /// Applies a freshly acquired DHCP lease and clears the in-progress manager.
    pub(super) async fn on_dhcp_acquired(&mut self, lease: DhcpLease) {
        self.dhcp = None;
        let index = self.snapshot.index;
        if let Err(e) = self.commit_lease(index, lease).await {
            kmsg::warn!(
                "Failed to apply DHCP lease on {}: {}",
                self.snapshot.name,
                e
            );
            self.set_state(InterfaceState::Failed);
            return;
        }
        self.set_state(InterfaceState::Configured);
        if let Some(l) = &self.snapshot.lease {
            kmsg::info!("DHCP acquired on {}: {}", self.snapshot.name, l.assigned_ip);
        }
    }

    /// Re-applies an existing lease after a link-up event without a new DORA exchange.
    pub(super) async fn recover_with_lease(&mut self, lease: DhcpLease) {
        let index = self.snapshot.index;
        if let Err(e) = self.apply_lease(index, &lease).await {
            kmsg::warn!(
                "Failed to re-apply lease on link-up for {}: {}",
                self.snapshot.name,
                e
            );
            self.set_state(InterfaceState::Failed);
            return;
        }
        self.timers.arm(&lease);
        self.set_state(InterfaceState::Configured);
    }

    pub(super) async fn renew_lease<C: DhcpConnector>(&mut self, connector: &C) {
        kmsg::info!("DHCP RENEW for {}", self.snapshot.name);
        self.set_dhcp_state(DhcpState::Renewing);
        if let Err(e) = self.do_renew(connector).await {
            kmsg::warn!("DHCP RENEW failed for {}: {}", self.snapshot.name, e);
        }
    }

    async fn do_renew<C: DhcpConnector>(&mut self, connector: &C) -> Result<()> {
        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;
        let socket = connector
            .create_unicast(self.snapshot.name.as_str(), assigned_ip)
            .await?;
        let result = dhcp::client::renew(&socket, &mac, server_ip, assigned_ip).await;
        match result {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!(
                    "DHCP RENEW NAK for {}, returning to INIT",
                    self.snapshot.name
                );
                self.do_full_dora(connector).await
            }
            Err(e) => Err(e),
        }
    }

    pub(super) async fn rebind_lease<C: DhcpConnector>(&mut self, connector: &C) {
        kmsg::info!("DHCP REBIND for {}", self.snapshot.name);
        self.set_dhcp_state(DhcpState::Rebinding);
        if let Err(e) = self.do_rebind(connector).await {
            kmsg::warn!("DHCP REBIND failed for {}: {}", self.snapshot.name, e);
        }
    }

    async fn do_rebind<C: DhcpConnector>(&mut self, connector: &C) -> Result<()> {
        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;
        let socket = connector.create_raw(self.snapshot.name.as_str()).await?;
        let result = dhcp::client::rebind(&socket, &mac, server_ip, assigned_ip).await;
        match result {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!(
                    "DHCP REBIND NAK for {}, returning to INIT",
                    self.snapshot.name
                );
                self.do_full_dora(connector).await
            }
            Err(e) => Err(e),
        }
    }

    /// Re-runs a full DORA exchange to recover from a NAK or lease expiry.
    pub(super) async fn do_full_dora<C: DhcpConnector>(&mut self, connector: &C) -> Result<()> {
        kmsg::warn!("DHCP re-acquiring on {}", self.snapshot.name);
        self.set_dhcp_state(DhcpState::Init);
        self.set_state(InterfaceState::Configuring);
        self.timers.disarm();

        let mac = self.snapshot.mac;
        let socket = connector.create_raw(self.snapshot.name.as_str()).await?;
        let lease = dhcp::client::run(&socket, &mac).await.inspect_err(|_| {
            self.set_state(InterfaceState::Failed);
        })?;

        let index = self.snapshot.index;
        self.commit_lease(index, lease).await?;
        self.set_state(InterfaceState::Configured);

        if let Some(l) = &self.snapshot.lease {
            kmsg::info!(
                "DHCP re-acquired on {}: {}",
                self.snapshot.name,
                l.assigned_ip
            );
        }
        Ok(())
    }

    /// Applies a renewed lease without changing the interface state.
    async fn apply_renewed_lease(&mut self, lease: &DhcpLease) -> Result<()> {
        let index = self.snapshot.index;
        self.commit_lease(index, lease.clone()).await?;
        kmsg::info!("DHCP lease renewed for {}", self.snapshot.name);
        Ok(())
    }

    /// Applies kernel-level changes, stores the lease, advances DHCP state, and arms timers.
    async fn commit_lease(&mut self, index: u32, lease: DhcpLease) -> Result<()> {
        self.apply_lease(index, &lease).await?;
        self.store_lease(&lease);
        self.set_dhcp_state(DhcpState::Bound);
        self.timers.arm(&lease);
        Ok(())
    }

    /// Applies the network-level changes from a lease.
    pub(super) async fn apply_lease(&mut self, index: u32, lease: &DhcpLease) -> Result<()> {
        self.ops
            .ensure_ipv4(index, lease.assigned_ip, lease.prefix_len)
            .await?;

        if let Some(gw) = lease.gateway {
            self.ops.ensure_default_route(gw).await?;
        } else {
            kmsg::info!(
                "No gateway in DHCP lease on {}, skipping default route",
                self.snapshot.name
            );
        }

        Ok(())
    }

    /// Persists the lease into the snapshot and publishes it.
    pub(super) fn store_lease(&mut self, lease: &DhcpLease) {
        self.snapshot.ip = Some(IpConfig {
            address: lease.assigned_ip,
            prefix_len: lease.prefix_len,
            gateway: lease.gateway,
            dns: lease.dns_servers.clone(),
        });
        self.snapshot.lease = Some(lease.clone());
        self.publish_snapshot();
    }

    pub(super) fn set_dhcp_state(&mut self, next: DhcpState) {
        let Some(current) = self.snapshot.dhcp_state.as_mut() else {
            self.snapshot.dhcp_state = Some(next);
            return;
        };
        if let Err(e) = current.transition(next) {
            kmsg::warn!(
                "DHCP state transition rejected on {}: {}",
                self.snapshot.name,
                e
            );
        }
    }

    fn extract_dhcp_params(&self) -> Result<([u8; 6], Ipv4Addr, Ipv4Addr)> {
        let lease = self
            .snapshot
            .lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", self.snapshot.name))?;
        Ok((self.snapshot.mac, lease.server_ip, lease.assigned_ip))
    }
}

fn deadline_to_sleep(now: SystemTime, deadline: SystemTime) -> Option<Pin<Box<Sleep>>> {
    let dur = deadline.duration_since(now).ok()?;
    Some(Box::pin(tokio::time::sleep(dur)))
}
