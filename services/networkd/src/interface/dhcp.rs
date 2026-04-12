//! DHCP lease life cycle management for a per-interface actor.

use anyhow::Result;
use netlib::address::IpConfig;
use netlib::{address, link, route};
use tokio::sync::mpsc;

use super::InterfaceActor;
use super::commands::{InterfaceCommand, LeaseAction};
use crate::dhcp::client::{rebind_dhcp_client, renew_dhcp_client, run_dhcp_client};
use crate::dhcp::codec::DhcpNak;
use crate::dhcp::{DhcpLease, DhcpState};
use crate::snapshot::{InterfaceSnapshot, InterfaceState};

impl InterfaceActor {
    /// Performs the initial DHCPDISCOVER->ACK exchange and applies the lease.
    pub(super) async fn acquire_dhcp(&mut self) -> Result<InterfaceSnapshot> {
        let iface = self.snapshot.name.to_string();
        self.set_state(InterfaceState::Configuring);

        let index = link::ensure_up(&self.handle, &iface).await?;
        let mac = self.snapshot.mac;
        let lease = run_dhcp_client(&iface, &mac).await.inspect_err(|_| {
            self.set_state(InterfaceState::Failed);
        })?;

        self.apply_lease(index, &lease).await?;
        self.store_lease(&lease);
        self.set_dhcp_state(DhcpState::Bound);
        self.set_state(InterfaceState::Configured);

        self.schedule_lease_renewal(&lease);

        kmsg::info!("DHCP acquired on {}: {}", iface, lease.assigned_ip);

        Ok(self.snapshot.clone())
    }

    /// Dispatches to the appropriate handler based on the lease timer phase.
    pub(super) async fn handle_lease_action(&mut self, action: LeaseAction) {
        let iface = self.snapshot.name.to_string();
        let result = match action {
            LeaseAction::Renew => self.renew_lease().await,
            LeaseAction::Rebind => self.rebind_lease().await,
            LeaseAction::Expired => self.handle_lease_expired().await,
        };
        if let Err(e) = result {
            kmsg::warn!("DHCP lease action failed for {}: {}", iface, e);
        }
    }

    async fn renew_lease(&mut self) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        kmsg::info!("DHCP RENEW for {}", iface);
        self.set_dhcp_state(DhcpState::Renewing);

        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;

        match renew_dhcp_client(&iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP RENEW NAK for {}, returning to INIT", iface);
                self.do_full_dora().await
            }
            Err(e) => {
                kmsg::warn!("DHCP RENEW failed for {}: {}", iface, e);
                Err(e)
            }
        }
    }

    async fn rebind_lease(&mut self) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        kmsg::info!("DHCP REBIND for {}", iface);
        self.set_dhcp_state(DhcpState::Rebinding);

        let (mac, server_ip, assigned_ip) = self.extract_dhcp_params()?;

        match rebind_dhcp_client(&iface, &mac, server_ip, assigned_ip).await {
            Ok(lease) => self.apply_renewed_lease(&lease).await,
            Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
                kmsg::warn!("DHCP REBIND NAK for {}, returning to INIT", iface);
                self.do_full_dora().await
            }
            Err(e) => {
                kmsg::warn!("DHCP REBIND failed for {}: {}", iface, e);
                Err(e)
            }
        }
    }

    async fn handle_lease_expired(&mut self) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        kmsg::warn!("DHCP lease expired for {}, re-acquiring", iface);
        self.do_full_dora().await
    }

    async fn do_full_dora(&mut self) -> Result<()> {
        let iface = self.snapshot.name.to_string();
        self.set_dhcp_state(DhcpState::Init);
        self.set_state(InterfaceState::Configuring);
        self.cancel_renewal_tasks();

        let mac = self.snapshot.mac;
        let lease = run_dhcp_client(&iface, &mac).await.inspect_err(|_| {
            self.set_state(InterfaceState::Failed);
        })?;

        let index = self.snapshot.index;
        self.apply_lease(index, &lease).await?;
        self.store_lease(&lease);
        self.set_dhcp_state(DhcpState::Bound);
        self.set_state(InterfaceState::Configured);

        self.schedule_lease_renewal(&lease);

        kmsg::info!("DHCP re-acquired on {}: {}", iface, lease.assigned_ip);
        Ok(())
    }

    async fn apply_renewed_lease(&mut self, lease: &DhcpLease) -> Result<()> {
        let index = self.snapshot.index;
        self.apply_lease(index, lease).await?;
        self.store_lease(lease);
        self.set_dhcp_state(DhcpState::Bound);
        self.cancel_renewal_tasks();
        self.schedule_lease_renewal(lease);

        kmsg::info!("DHCP lease renewed for {}", self.snapshot.name);
        Ok(())
    }

    async fn apply_lease(&mut self, index: u32, lease: &DhcpLease) -> Result<()> {
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
            self.update_dns_v4(dns)?;
        }

        Ok(())
    }

    fn store_lease(&mut self, lease: &DhcpLease) {
        self.snapshot.ip = Some(IpConfig {
            address: lease.assigned_ip,
            prefix_len: lease.prefix_len,
            gateway: lease.gateway,
            dns: lease.dns_servers.clone(),
        });
        self.snapshot.lease = Some(lease.clone());
        self.publish_snapshot();
    }

    fn set_dhcp_state(&mut self, state: DhcpState) {
        self.snapshot.dhcp_state = Some(state);
    }

    fn extract_dhcp_params(&self) -> Result<([u8; 6], std::net::Ipv4Addr, std::net::Ipv4Addr)> {
        let lease = self
            .snapshot
            .lease
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", self.snapshot.name))?;
        Ok((self.snapshot.mac, lease.server_ip, lease.assigned_ip))
    }

    pub(super) fn schedule_lease_renewal(&mut self, lease: &DhcpLease) {
        self.cancel_renewal_tasks();

        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let cmd_tx = self.self_tx.clone();
        let renew_task = spawn_lease_task(cmd_tx.clone(), renew_deadline, LeaseAction::Renew);
        let rebind_task = spawn_lease_task(cmd_tx.clone(), rebind_deadline, LeaseAction::Rebind);
        let expiry_task = spawn_lease_task(cmd_tx, expiry_deadline, LeaseAction::Expired);

        self.renewal_tasks.push(renew_task);
        self.renewal_tasks.push(rebind_task);
        self.renewal_tasks.push(expiry_task);
    }

    pub(super) fn cancel_renewal_tasks(&mut self) {
        for task in self.renewal_tasks.drain(..) {
            task.abort();
        }
    }
}

fn spawn_lease_task(
    cmd_tx: mpsc::Sender<InterfaceCommand>,
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
        kmsg::info!("Lease {} timer fired", label);
        let _ = cmd_tx.send(InterfaceCommand::LeaseAction(action)).await;
    })
}
