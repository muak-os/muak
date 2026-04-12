//! DHCP lease life cycle management for a per-interface actor.

use std::time::SystemTime;

use anyhow::Result;
use netlib::address::IpConfig;
use netlib::{address, route};

use super::InterfaceActor;
use crate::dhcp::client::{rebind_dhcp_client, renew_dhcp_client, run_dhcp_client};
use crate::dhcp::codec::DhcpNak;
use crate::dhcp::{DhcpLease, DhcpManager, DhcpState};
use crate::interface::state::InterfaceState;
use crate::state_machine::StateMachine;

impl InterfaceActor {
    /// Initialises a `DhcpManager` and marks the interface as configuring.
    pub(super) fn start_dhcp(&mut self) {
        self.set_state(InterfaceState::Configuring);
        self.dhcp = Some(DhcpManager::new(
            self.snapshot.name.to_string(),
            self.snapshot.mac,
        ));
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
        kmsg::info!(
            "DHCP acquired on {}: {}",
            self.snapshot.name,
            self.snapshot
                .lease
                .as_ref()
                .map(|l| l.assigned_ip.to_string())
                .unwrap_or_default()
        );
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
        self.arm_lease_timers(&lease);
        self.set_state(InterfaceState::Configured);
    }

    pub(super) async fn renew_lease(&mut self) {
        let iface = self.snapshot.name.to_string();
        kmsg::info!("DHCP RENEW for {}", iface);
        self.set_dhcp_state(DhcpState::Renewing);
        if let Err(e) = self.do_renew(&iface).await {
            kmsg::warn!("DHCP RENEW failed for {}: {}", iface, e);
        }
    }

    async fn do_renew(&mut self, iface: &str) -> Result<()> {
        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;
        match renew_dhcp_client(iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP RENEW NAK for {}, returning to INIT", iface);
                self.do_full_dora().await
            }
            Err(e) => Err(e),
        }
    }

    pub(super) async fn rebind_lease(&mut self) {
        let iface = self.snapshot.name.to_string();
        kmsg::info!("DHCP REBIND for {}", iface);
        self.set_dhcp_state(DhcpState::Rebinding);
        if let Err(e) = self.do_rebind(&iface).await {
            kmsg::warn!("DHCP REBIND failed for {}: {}", iface, e);
        }
    }

    async fn do_rebind(&mut self, iface: &str) -> Result<()> {
        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;
        match rebind_dhcp_client(iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP REBIND NAK for {}, returning to INIT", iface);
                self.do_full_dora().await
            }
            Err(e) => Err(e),
        }
    }

    /// Re-runs a full DORA exchange to recover from a NAK or lease expiry.
    pub(super) async fn do_full_dora(&mut self) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        kmsg::warn!("DHCP re-acquiring on {}", iface);
        self.set_dhcp_state(DhcpState::Init);
        self.set_state(InterfaceState::Configuring);
        self.disarm_lease_timers();

        let mac = self.snapshot.mac;
        let lease = run_dhcp_client(&iface, &mac).await.inspect_err(|_| {
            self.set_state(InterfaceState::Failed);
        })?;

        let index = self.snapshot.index;
        self.commit_lease(index, lease).await?;
        self.set_state(InterfaceState::Configured);

        kmsg::info!(
            "DHCP re-acquired on {}: {}",
            iface,
            self.snapshot
                .lease
                .as_ref()
                .map(|l| l.assigned_ip.to_string())
                .unwrap_or_default()
        );
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
        self.arm_lease_timers(&lease);
        Ok(())
    }

    /// Applies the network-level changes from a lease: address, route, and DNS.
    pub(super) async fn apply_lease(&mut self, index: u32, lease: &DhcpLease) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        address::ensure_ipv4(&self.handle, index, lease.assigned_ip, lease.prefix_len).await?;

        if let Some(gw) = lease.gateway {
            kmsg::info!("Setting default route via {}", gw);
            route::ensure_default_route(&self.handle, gw).await?;
        } else {
            kmsg::info!(
                "No gateway in DHCP lease on {}, skipping default route",
                iface
            );
        }

        let dns = if lease.dns_servers.is_empty() {
            config::network().ipv4_dns()
        } else {
            lease.dns_servers.clone()
        };

        if lease.dns_servers.is_empty() && !dns.is_empty() {
            kmsg::info!(
                "No DNS from DHCP, using {} configured fallback server(s)",
                dns.len()
            );
        }

        if !dns.is_empty() {
            self.dns.update_v4(dns)?;
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

    fn extract_dhcp_params(&self) -> Result<([u8; 6], std::net::Ipv4Addr, std::net::Ipv4Addr)> {
        let lease = self
            .snapshot
            .lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", self.snapshot.name))?;
        Ok((self.snapshot.mac, lease.server_ip, lease.assigned_ip))
    }

    /// Arms the renew, rebind, and expiry sleep timers from a newly acquired lease.
    pub(super) fn arm_lease_timers(&mut self, lease: &DhcpLease) {
        self.disarm_lease_timers();

        let now = SystemTime::now();
        self.renew_at = deadline_to_sleep(now, lease.obtained_at + lease.renewal_time);
        self.rebind_at = deadline_to_sleep(now, lease.obtained_at + lease.rebind_time);
        self.expire_at = deadline_to_sleep(now, lease.expiry());
    }

    /// Cancels all active lease timers.
    pub(super) fn disarm_lease_timers(&mut self) {
        self.renew_at = None;
        self.rebind_at = None;
        self.expire_at = None;
    }
}

fn deadline_to_sleep(
    now: SystemTime,
    deadline: SystemTime,
) -> Option<std::pin::Pin<Box<tokio::time::Sleep>>> {
    let dur = deadline.duration_since(now).ok()?;
    Some(Box::pin(tokio::time::sleep(dur)))
}
