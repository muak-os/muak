use anyhow::Result;
use tokio::sync::mpsc;

use crate::log;
use crate::network::bridge::attach_to_bridge;
use crate::network::bridge::ensure_bridge_with_ip_transfer;
use crate::network::config::LAN_BRIDGE_NAME;
use crate::network::dhcp::run_dhcp_client;
use crate::network::dns::configure_dns;
use crate::network::interface::{LinkState as OldLinkState, discover_ethernet_interfaces};
use crate::network::model::{DhcpLease, InterfaceSnapshot, LinkStateKind, NetworkStateKind};
use crate::network::ops::{ensure_addr, ensure_default_route_v4, ensure_link_up};
use crate::network::tap::{bring_up_tap, create_tap, delete_tap};

use super::commands::NetworkCommand;
use super::state::NetworkActor;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        log!("network", "Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dhcp_on_primary(cmd_tx).await?;
        self.setup_bridge().await?;

        self.state.state = NetworkStateKind::Ready;
        self.publish_state();
        log!("network", "Network initialization complete");

        Ok(())
    }

    async fn discover_interfaces(&mut self) -> Result<()> {
        log!("network", "Discovering ethernet interfaces");
        self.state.state = NetworkStateKind::Initializing;
        self.publish_state();

        let discovered = discover_ethernet_interfaces(&self.handle).await?;
        if discovered.is_empty() {
            self.state.state = NetworkStateKind::Degraded;
            self.publish_state();
            anyhow::bail!("no ethernet interfaces found");
        }

        self.populate_interface_map(&discovered);
        self.select_primary_interface(&discovered);

        self.state.state = NetworkStateKind::Operational;
        self.sync_and_publish();
        log!(
            "network",
            "Discovered {} interfaces, primary={:?}",
            discovered.len(),
            self.state.primary
        );

        Ok(())
    }

    fn populate_interface_map(&mut self, discovered: &[crate::network::interface::Interface]) {
        for iface in discovered {
            let snapshot = InterfaceSnapshot {
                name: iface.name.clone(),
                index: iface.index,
                mac: iface.mac_address,
                link: match iface.link_state {
                    OldLinkState::Up => LinkStateKind::Up,
                    OldLinkState::Down => LinkStateKind::Down,
                },
                ip: None,
                lease: None,
            };
            self.insert_interface(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[crate::network::interface::Interface]) {
        let primary = discovered
            .iter()
            .find(|i| i.link_state == OldLinkState::Up)
            .unwrap_or(&discovered[0]);

        self.state.primary = Some(primary.name.clone());
        self.state.backups = discovered
            .iter()
            .filter(|i| i.name != primary.name)
            .map(|i| i.name.clone())
            .collect();

        log!(
            "network",
            "Selected primary: {}, backups: {:?}",
            primary.name,
            self.state.backups
        );
    }

    async fn acquire_dhcp_on_primary(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self
            .state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))?;

        log!(
            "network",
            "Acquiring DHCP on primary interface: {}",
            primary
        );
        self.acquire_dhcp(&primary, cmd_tx).await?;
        Ok(())
    }

    pub(super) async fn acquire_dhcp(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        log!("network", "Acquiring DHCP on {}", iface);

        let index = ensure_link_up(&self.handle, iface).await?;
        let mac = self
            .get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;

        let (ip_cfg, lease) = run_dhcp_client(iface, &mac).await?;

        self.apply_ip_configuration(index, &ip_cfg).await?;
        self.update_interface_with_lease(iface, ip_cfg.clone(), lease.clone())?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);

        log!("network", "DHCP acquired on {}: {}", iface, ip_cfg.address);

        Ok(self
            .get_interface(iface)
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))?
            .clone())
    }

    async fn apply_ip_configuration(
        &mut self,
        index: u32,
        ip_cfg: &crate::network::model::IpConfig,
    ) -> Result<()> {
        ensure_addr(&self.handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;

        if let Some(gw) = ip_cfg.gateway {
            ensure_default_route_v4(&self.handle, gw).await?;
        }

        if !ip_cfg.dns.is_empty() {
            configure_dns(&ip_cfg.dns)?;
        }

        Ok(())
    }

    fn update_interface_with_lease(
        &mut self,
        iface: &str,
        ip_cfg: crate::network::model::IpConfig,
        lease: DhcpLease,
    ) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ip = Some(ip_cfg);
        iface_snap.lease = Some(lease);
        self.sync_and_publish();

        Ok(())
    }

    fn schedule_lease_renewal(
        &self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        lease: DhcpLease,
    ) {
        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), renew_deadline, "renewal");
        Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), rebind_deadline, "rebind");
        Self::spawn_renewal_task(cmd_tx, iface, expiry_deadline, "expiry");
    }

    fn spawn_renewal_task(
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        deadline: std::time::SystemTime,
        task_type: &str,
    ) {
        let task_name = task_type.to_string();
        tokio::spawn(async move {
            let now = std::time::SystemTime::now();
            if let Ok(dur) = deadline.duration_since(now) {
                tokio::time::sleep(dur).await;
            } else {
                return;
            }

            log!("network", "Lease {} attempt for {}", task_name, iface);
            let _ = cmd_tx.send(NetworkCommand::RenewLease { iface }).await;
        });
    }

    pub(super) async fn renew_lease(&mut self, iface: &str) -> Result<()> {
        log!("network", "Renewing DHCP lease for {}", iface);

        let mac = self
            .get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;

        match run_dhcp_client(iface, &mac).await {
            Ok((ip_cfg, lease)) => {
                let index = self
                    .get_interface(iface)
                    .ok_or_else(|| anyhow::anyhow!("interface disappeared"))?
                    .index;

                self.apply_ip_configuration(index, &ip_cfg).await?;
                self.update_interface_with_lease(iface, ip_cfg, lease)?;

                log!("network", "DHCP lease renewed for {}", iface);
                Ok(())
            }
            Err(e) => {
                log!("network", "DHCP renewal failed for {}: {}", iface, e);
                Err(anyhow::anyhow!("DHCP renewal failed: {}", e))
            }
        }
    }

    pub(super) async fn setup_bridge(&mut self) -> Result<()> {
        let primary = self
            .state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))?;

        log!(
            "network",
            "Setting up bridge {} with primary {}",
            LAN_BRIDGE_NAME,
            primary
        );

        ensure_bridge_with_ip_transfer(&self.handle, LAN_BRIDGE_NAME, &primary).await?;

        log!(
            "network",
            "Bridge setup complete: {} <- {}",
            LAN_BRIDGE_NAME,
            primary
        );

        Ok(())
    }

    pub(super) async fn add_tap(&mut self, name: &str) -> Result<InterfaceSnapshot> {
        log!("network", "Adding TAP interface: {}", name);

        create_tap(name).await?;
        bring_up_tap(&self.handle, name).await?;
        attach_to_bridge(&self.handle, name, LAN_BRIDGE_NAME).await?;

        let index = ensure_link_up(&self.handle, name).await?;
        let snapshot = InterfaceSnapshot {
            name: name.to_string(),
            index,
            mac: [0, 0, 0, 0, 0, 0],
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
        };

        self.insert_interface(snapshot.clone());
        self.sync_and_publish();

        log!("network", "TAP interface added: {}", name);
        Ok(snapshot)
    }

    pub(super) async fn delete_tap(&mut self, name: &str) -> Result<()> {
        log!("network", "Deleting TAP interface: {}", name);

        delete_tap(&self.handle, name).await?;
        self.remove_interface(name);
        self.sync_and_publish();

        log!("network", "TAP interface deleted: {}", name);
        Ok(())
    }
}
