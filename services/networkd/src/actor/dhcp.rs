//! DHCP lease lifecycle management for the network actor.

use anyhow::Result;
use netlib::address::IpConfig;
use netlib::{address, link, route};
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::{InterfaceSnapshot, NetworkActor};
use crate::dhcp::{DhcpLease, client::run_dhcp_client};
use crate::dns::configure_dns;

impl NetworkActor {
    pub(super) async fn acquire_dhcp(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        let index = link::ensure_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;
        let (ip, lease) = run_dhcp_client(iface, &mac).await?;

        self.apply_ip_configuration(index, &ip).await?;
        self.update_interface_with_lease(iface, ip.clone(), lease.clone())?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);

        kmsg::info!("DHCP acquired on {}: {}", iface, ip.address);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared after DHCP on {}", iface))
    }

    pub(super) async fn renew_lease(&mut self, iface: &str) -> Result<()> {
        kmsg::info!("Renewing DHCP lease for {}", iface);

        let mac = self.get_interface_mac(iface)?;

        let (ip, lease) = match run_dhcp_client(iface, &mac).await {
            Ok(result) => result,
            Err(e) => {
                kmsg::warn!("DHCP renewal failed for {}: {}", iface, e);
                return Err(anyhow::anyhow!("DHCP renewal failed for {}: {}", iface, e));
            }
        };

        let index = self.get_interface(iface).map(|i| i.index).ok_or_else(|| {
            anyhow::anyhow!("interface disappeared during DHCP renewal: {}", iface)
        })?;

        self.apply_ip_configuration(index, &ip).await?;
        self.update_interface_with_lease(iface, ip, lease)?;

        kmsg::info!("DHCP lease renewed for {}", iface);
        Ok(())
    }

    pub(super) async fn apply_ip_configuration(&mut self, index: u32, ip: &IpConfig) -> Result<()> {
        address::ensure_ipv4(&self.handle, index, ip.address, ip.prefix_len).await?;

        if let Some(gw) = ip.gateway {
            kmsg::info!("Setting default route via {}", gw);
            route::ensure_default_route(&self.handle, gw).await?;
        } else {
            kmsg::info!("No gateway in DHCP lease, skipping default route");
        }

        let dns = if ip.dns.is_empty() {
            config::network().ipv4_dns()
        } else {
            ip.dns.clone()
        };

        if ip.dns.is_empty() && !dns.is_empty() {
            kmsg::info!(
                "No DNS from DHCP, using {} configured fallback server(s)",
                dns.len()
            );
        }

        if !dns.is_empty() {
            configure_dns(&dns)?;
        }

        Ok(())
    }

    pub(super) fn update_interface_with_lease(
        &mut self,
        iface: &str,
        ip: IpConfig,
        lease: DhcpLease,
    ) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ip = Some(ip);
        iface_snap.lease = Some(lease);
        self.sync_and_publish();

        Ok(())
    }

    pub(super) fn schedule_lease_renewal(
        &mut self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        lease: DhcpLease,
    ) {
        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let renew_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), renew_deadline, "renewal");
        let rebind_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), rebind_deadline, "rebind");
        let expiry_task =
            Self::spawn_renewal_task(cmd_tx, iface.clone(), expiry_deadline, "expiry");

        self.track_renewal_task(iface.clone(), renew_task);
        self.track_renewal_task(iface.clone(), rebind_task);
        self.track_renewal_task(iface, expiry_task);
    }

    fn spawn_renewal_task(
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        deadline: std::time::SystemTime,
        task_type: &str,
    ) -> tokio::task::JoinHandle<()> {
        let task_name = task_type.to_string();
        let now = std::time::SystemTime::now();
        let Some(dur) = deadline.duration_since(now).ok() else {
            return tokio::spawn(std::future::ready(()));
        };

        tokio::spawn(async move {
            tokio::time::sleep(dur).await;
            kmsg::info!("Lease {} attempt for {}", task_name, iface);
            let _ = cmd_tx.send(NetworkCommand::RenewLease { iface }).await;
        })
    }

    pub(super) fn get_interface_mac(&self, iface: &str) -> Result<[u8; 6]> {
        self.get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))
    }
}
