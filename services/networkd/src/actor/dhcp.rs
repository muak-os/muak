//! DHCP lease life cycle management for the network actor.

use anyhow::{Context, Result};
use netlib::address::IpConfig;
use netlib::interface::InterfaceName;
use netlib::{address, link, route};
use tokio::sync::mpsc;

use super::commands::{LeaseAction, NetworkCommand};
use super::state::{InterfaceSnapshot, InterfaceState, NetworkActor};
use crate::dhcp::client::{rebind_dhcp_client, renew_dhcp_client, run_dhcp_client};
use crate::dhcp::codec::DhcpNak;
use crate::dhcp::{DhcpLease, DhcpState};

impl NetworkActor {
    /// Performs the initial DHCPDISCOVER->ACK exchange and applies the lease.
    pub(super) async fn acquire_dhcp(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        self.set_interface_state(iface, InterfaceState::Configuring);

        let index = link::ensure_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;
        let lease = run_dhcp_client(iface, &mac).await.inspect_err(|_| {
            self.set_interface_state(iface, InterfaceState::Failed);
        })?;

        self.apply_lease(index, iface, &lease).await?;
        self.store_lease(iface, &lease)?;
        self.set_dhcp_state(iface, DhcpState::Bound);
        self.set_interface_state(iface, InterfaceState::Configured);

        let iface_name = InterfaceName::new(iface)
            .with_context(|| format!("invalid interface name: {iface}"))?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface_name, &lease);

        kmsg::info!("DHCP acquired on {}: {}", iface, lease.assigned_ip);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared after DHCP on {}", iface))
    }

    /// Dispatches to the appropriate handler based on the lease timer phase.
    pub(super) async fn handle_lease_action(
        &mut self,
        iface: &str,
        action: LeaseAction,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) {
        let result = match action {
            LeaseAction::Renew => self.renew_lease(iface, cmd_tx).await,
            LeaseAction::Rebind => self.rebind_lease(iface, cmd_tx).await,
            LeaseAction::Expired => self.handle_lease_expired(iface, cmd_tx).await,
        };
        if let Err(e) = result {
            kmsg::warn!("DHCP lease action failed for {}: {}", iface, e);
        }
    }

    /// Attempts a unicast RENEW to the known DHCP server.
    async fn renew_lease(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        kmsg::info!("DHCP RENEW for {}", iface);
        self.set_dhcp_state(iface, DhcpState::Renewing);

        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params(iface)?;

        match renew_dhcp_client(iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(iface, &lease, cmd_tx).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP RENEW NAK for {}, returning to INIT", iface);
                self.do_full_dora(iface, cmd_tx).await
            }
            Err(e) => {
                kmsg::warn!("DHCP RENEW failed for {}: {}", iface, e);
                Err(e)
            }
        }
    }

    /// Attempts a broadcast REBIND to any available server.
    async fn rebind_lease(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        kmsg::info!("DHCP REBIND for {}", iface);
        self.set_dhcp_state(iface, DhcpState::Rebinding);

        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params(iface)?;

        match rebind_dhcp_client(iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(iface, &lease, cmd_tx).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP REBIND NAK for {}, returning to INIT", iface);
                self.do_full_dora(iface, cmd_tx).await
            }
            Err(e) => {
                kmsg::warn!("DHCP REBIND failed for {}: {}", iface, e);
                Err(e)
            }
        }
    }

    /// Lease has expired; perform a full DORA from scratch.
    async fn handle_lease_expired(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        kmsg::warn!("DHCP lease expired for {}, re-acquiring", iface);
        self.do_full_dora(iface, cmd_tx).await
    }

    /// Performs a full DISCOVER->OFFER->REQUEST->ACK exchange (INIT state).
    async fn do_full_dora(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        self.set_dhcp_state(iface, DhcpState::Init);
        self.set_interface_state(iface, InterfaceState::Configuring);
        self.cancel_renewal_tasks(iface);

        let mac = self.get_interface_mac(iface)?;
        let lease = run_dhcp_client(iface, &mac).await.inspect_err(|_| {
            self.set_interface_state(iface, InterfaceState::Failed);
        })?;

        let index = self.get_interface(iface).map(|i| i.index).ok_or_else(|| {
            anyhow::anyhow!("interface disappeared during DHCP re-acquire: {}", iface)
        })?;

        self.apply_lease(index, iface, &lease).await?;
        self.store_lease(iface, &lease)?;
        self.set_dhcp_state(iface, DhcpState::Bound);
        self.set_interface_state(iface, InterfaceState::Configured);

        let iface_name = InterfaceName::new(iface)
            .with_context(|| format!("invalid interface name: {iface}"))?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface_name, &lease);

        kmsg::info!("DHCP re-acquired on {}: {}", iface, lease.assigned_ip);
        Ok(())
    }

    /// Applies a successful renewal/rebind: updates IP, stores lease, reschedules timers.
    async fn apply_renewed_lease(
        &mut self,
        iface: &str,
        lease: &DhcpLease,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let index = self.get_interface(iface).map(|i| i.index).ok_or_else(|| {
            anyhow::anyhow!("interface disappeared during DHCP renewal: {}", iface)
        })?;

        self.apply_lease(index, iface, lease).await?;
        self.store_lease(iface, lease)?;
        self.set_dhcp_state(iface, DhcpState::Bound);
        self.cancel_renewal_tasks(iface);
        let iface_name = InterfaceName::new(iface)
            .with_context(|| format!("invalid interface name: {iface}"))?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface_name, lease);

        kmsg::info!("DHCP lease renewed for {}", iface);
        Ok(())
    }

    /// Applies the IP configuration from a DHCP lease to the system.
    async fn apply_lease(&mut self, index: u32, iface: &str, lease: &DhcpLease) -> Result<()> {
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
            self.update_dns_v4(dns)?;
        }

        Ok(())
    }

    /// Stores the lease and its derived IpConfig on the interface snapshot.
    fn store_lease(&mut self, iface: &str, lease: &DhcpLease) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ip = Some(IpConfig {
            address: lease.assigned_ip,
            prefix_len: lease.prefix_len,
            gateway: lease.gateway,
            dns: lease.dns_servers.clone(),
        });
        iface_snap.lease = Some(lease.clone());
        self.sync_and_publish();

        Ok(())
    }

    fn set_dhcp_state(&mut self, iface: &str, state: DhcpState) {
        if let Some(snap) = self.get_interface_mut(iface) {
            snap.dhcp_state = Some(state);
        }
    }

    fn extract_dhcp_params(
        &self,
        iface: &str,
    ) -> Result<([u8; 6], std::net::Ipv4Addr, std::net::Ipv4Addr)> {
        let snap = self
            .get_interface(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;
        let lease = snap
            .lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", iface))?;
        Ok((snap.mac, lease.server_ip, lease.assigned_ip))
    }

    /// Schedules three distinct timer tasks for RENEW, REBIND, and EXPIRY.
    pub(super) fn schedule_lease_renewal(
        &mut self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: InterfaceName,
        lease: &DhcpLease,
    ) {
        self.cancel_renewal_tasks(iface.as_str());

        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let iface_str = iface.to_string();
        let renew_task = Self::spawn_lease_task(
            cmd_tx.clone(),
            iface_str.clone(),
            renew_deadline,
            LeaseAction::Renew,
        );
        let rebind_task = Self::spawn_lease_task(
            cmd_tx.clone(),
            iface_str.clone(),
            rebind_deadline,
            LeaseAction::Rebind,
        );
        let expiry_task =
            Self::spawn_lease_task(cmd_tx, iface_str, expiry_deadline, LeaseAction::Expired);

        self.track_renewal_task(iface.clone(), renew_task);
        self.track_renewal_task(iface.clone(), rebind_task);
        self.track_renewal_task(iface, expiry_task);
    }

    fn spawn_lease_task(
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        deadline: std::time::SystemTime,
        action: LeaseAction,
    ) -> tokio::task::JoinHandle<()> {
        let now = std::time::SystemTime::now();
        let Some(dur) = deadline.duration_since(now).ok() else {
            return tokio::spawn(std::future::ready(()));
        };

        let label = format!("{:?}", action);
        tokio::spawn(async move {
            tokio::time::sleep(dur).await;
            kmsg::info!("Lease {} timer fired for {}", label, iface);
            let _ = cmd_tx
                .send(NetworkCommand::DhcpLeaseAction { iface, action })
                .await;
        })
    }

    pub(super) fn get_interface_mac(&self, iface: &str) -> Result<[u8; 6]> {
        self.get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))
    }
}
