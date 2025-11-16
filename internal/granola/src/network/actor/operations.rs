use anyhow::Result;
use tokio::sync::mpsc;

use crate::log;
use crate::network::config::LAN_BRIDGE_NAME;
use crate::network::dhcp::run_dhcp_client;
use crate::network::dns::configure_dns;
use crate::network::interface::InterfaceSelector;
use crate::network::interface::{LinkState as OldLinkState, discover_ethernet_interfaces};
use crate::network::model::{DhcpLease, InterfaceSnapshot, LinkStateKind, NetworkStateKind};
use crate::network::netlink::{address, link, route};
use crate::network::services::{bridge, tap};

use super::commands::NetworkCommand;
use super::state::NetworkActor;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        log!("network", "Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dhcp_on_primary(cmd_tx).await?;
        self.setup_bridge_and_transfer_dhcp(cmd_tx).await?;

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
        let primary = InterfaceSelector::select_primary(discovered)
            .expect("BUG: select_primary_interface called with empty list");

        self.state.primary = Some(primary.name.clone());

        let backups = InterfaceSelector::select_backups(discovered, &primary.name);
        self.state.backups = backups.iter().map(|i| i.name.clone()).collect();

        log!(
            "network",
            "Selected primary: {} (link: {}, priority: best), backups: {:?}",
            primary.name,
            primary.link_state,
            self.state.backups
        );
    }

    async fn acquire_dhcp_on_primary(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;

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

        let index = link::ensure_link_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;
        let (ip_cfg, lease) = run_dhcp_client(iface, &mac).await?;

        self.apply_ip_configuration(index, &ip_cfg).await?;
        self.update_interface_with_lease(iface, ip_cfg.clone(), lease.clone())?;
        self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);

        log!("network", "DHCP acquired on {}: {}", iface, ip_cfg.address);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))
    }

    fn get_interface_mac(&self, iface: &str) -> Result<[u8; 6]> {
        self.get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))
    }

    async fn apply_ip_configuration(
        &mut self,
        index: u32,
        ip_cfg: &crate::network::model::IpConfig,
    ) -> Result<()> {
        address::ensure_ipv4(&self.handle, index, ip_cfg.address, ip_cfg.prefix_len).await?;

        if let Some(gw) = ip_cfg.gateway {
            route::ensure_default_route(&self.handle, gw).await?;
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
        &mut self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        lease: DhcpLease,
    ) {
        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let renew_deadline_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), renew_deadline, "renewal");
        let rebind_deadline_task =
            Self::spawn_renewal_task(cmd_tx.clone(), iface.clone(), rebind_deadline, "rebind");
        let expiry_deadline_task =
            Self::spawn_renewal_task(cmd_tx, iface.clone(), expiry_deadline, "expiry");

        self.track_renewal_task(iface.clone(), renew_deadline_task);
        self.track_renewal_task(iface.clone(), rebind_deadline_task);
        self.track_renewal_task(iface, expiry_deadline_task);
    }

    fn spawn_renewal_task(
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        deadline: std::time::SystemTime,
        task_type: &str,
    ) -> tokio::task::JoinHandle<()> {
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
        })
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
        let primary = self.get_primary_name()?;

        log!(
            "network",
            "Setting up bridge {} with primary {}",
            LAN_BRIDGE_NAME,
            primary
        );
        bridge::ensure_bridge_with_ip_transfer(&self.handle, LAN_BRIDGE_NAME, &primary).await?;
        log!(
            "network",
            "Bridge setup complete: {} <- {}",
            LAN_BRIDGE_NAME,
            primary
        );

        Ok(())
    }

    async fn setup_bridge_and_transfer_dhcp(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let (lease, mac) = self.extract_lease_and_mac(&primary)?;

        self.setup_bridge().await?;

        self.cancel_renewal_tasks(&primary);

        let br_index = self.lookup_bridge_index().await?;
        self.track_bridge_interface(br_index, mac, lease.clone());
        self.clear_lease_from_primary(&primary);
        self.sync_and_publish();

        log!(
            "network",
            "Transferring DHCP lease management from {} to {}",
            primary,
            LAN_BRIDGE_NAME
        );
        self.schedule_lease_renewal(cmd_tx.clone(), LAN_BRIDGE_NAME.to_string(), lease);

        Ok(())
    }

    fn get_primary_name(&self) -> Result<String> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    fn extract_lease_and_mac(&self, iface_name: &str) -> Result<(DhcpLease, [u8; 6])> {
        let iface = self
            .get_interface(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?;

        let lease = iface
            .lease
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", iface_name))?;

        Ok((lease, iface.mac))
    }

    async fn lookup_bridge_index(&self) -> Result<u32> {
        link::get_link_index(&self.handle, LAN_BRIDGE_NAME).await
    }

    fn track_bridge_interface(&mut self, index: u32, mac: [u8; 6], lease: DhcpLease) {
        let primary = self.state.primary.as_ref().unwrap();
        let ip = self.get_interface(primary).and_then(|i| i.ip.clone());

        let br_snapshot = InterfaceSnapshot {
            name: LAN_BRIDGE_NAME.to_string(),
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease),
        };

        self.insert_interface(br_snapshot);
    }

    fn clear_lease_from_primary(&mut self, primary: &str) {
        if let Some(iface) = self.get_interface_mut(primary) {
            iface.ip = None;
            iface.lease = None;
        }
    }

    pub(super) async fn add_tap(&mut self, name: &str) -> Result<InterfaceSnapshot> {
        log!("network", "Adding TAP interface: {}", name);

        let index = tap::setup_tap_on_bridge(&self.handle, name, LAN_BRIDGE_NAME).await?;

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

        tap::remove_tap_device(&self.handle, name).await?;
        self.remove_interface(name);
        self.sync_and_publish();

        log!("network", "TAP interface deleted: {}", name);
        Ok(())
    }
}
