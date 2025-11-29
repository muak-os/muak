use anyhow::Result;
use tokio::sync::mpsc;

use crate::log;
use crate::network::config::LAN_BRIDGE_NAME;
use crate::network::dhcp::run_dhcp_client;
use crate::network::dhcpv6::run_dhcpv6_client;
use crate::network::dns::{configure_dns, configure_dns_v6};
use crate::network::interface::InterfaceSelector;
use crate::network::interface::{LinkState as OldLinkState, discover_ethernet_interfaces};
use crate::network::model::{DhcpLease, InterfaceSnapshot, Ipv6Config, LinkStateKind, NetworkStateKind};
use crate::network::netlink::{address, link, route};
use crate::network::services::{bridge, tap};

use super::commands::NetworkCommand;
use super::state::NetworkActor;

impl NetworkActor {
    pub(super) async fn initialize(&mut self, cmd_tx: &mpsc::Sender<NetworkCommand>) -> Result<()> {
        log!("network", "Initializing network");

        self.discover_interfaces().await?;
        self.acquire_dual_stack_on_primary(cmd_tx).await?;
        self.setup_bridge_and_transfer_dhcp(cmd_tx).await?;

        self.state.state = NetworkStateKind::Ready;
        self.ready_at = Some(std::time::Instant::now());
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
                ipv4: None,
                ipv4_lease: None,
                ipv6: None,
                ipv6_lease: None,
                #[allow(deprecated)]
                ip: None,
                #[allow(deprecated)]
                lease: None,
            };
            self.insert_interface(snapshot);
        }
    }

    fn select_primary_interface(&mut self, discovered: &[crate::network::interface::Interface]) {
        let primary = InterfaceSelector::select_primary(discovered)
            .expect("BUG: select_primary_interface called with empty list");

        // Set both primary (preferred) and active (current) to the selected interface
        self.state.primary = Some(primary.name.clone());
        self.state.active = Some(primary.name.clone());

        // Remaining interfaces become secondaries (backups for failover)
        let secondaries = InterfaceSelector::select_secondaries(discovered, &primary.name);
        self.state.secondaries = secondaries.iter().map(|i| i.name.clone()).collect();

        log!(
            "network",
            "Selected primary: {} (link: {}, priority: best), secondaries: {:?}",
            primary.name,
            primary.link_state,
            self.state.secondaries
        );
    }

    async fn acquire_dual_stack_on_primary(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;

        log!(
            "network",
            "Acquiring dual-stack (IPv4 + IPv6) on primary interface: {}",
            primary
        );
        
        // Try to acquire both IPv4 and IPv6
        // At minimum, we need IPv4 to succeed for backward compatibility
        let result = self.acquire_dual_stack(&primary, cmd_tx).await?;
        
        // Check that we got at least IPv4
        if result.ipv4.is_none() && result.ip.is_none() {
            anyhow::bail!("Failed to acquire IPv4 address on primary interface");
        }
        
        Ok(())
    }

    #[allow(dead_code)]
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

        // Update new fields
        iface_snap.ipv4 = Some(ip_cfg.clone());
        iface_snap.ipv4_lease = Some(lease.clone());
        
        // Update deprecated fields for backward compatibility
        #[allow(deprecated)]
        {
            iface_snap.ip = Some(ip_cfg);
            iface_snap.lease = Some(lease);
        }
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

    // ========================================================================
    // DHCPv6 / IPv6 Operations
    // ========================================================================

    /// Acquire an IPv6 address via DHCPv6
    pub(super) async fn acquire_dhcpv6(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        log!("network", "Acquiring DHCPv6 on {}", iface);

        let index = link::ensure_link_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;

        let (ipv6_cfg, lease) = run_dhcpv6_client(iface, &mac).await?;

        self.apply_ipv6_configuration(index, &ipv6_cfg).await?;
        self.update_interface_with_ipv6_lease(iface, ipv6_cfg.clone(), lease.clone())?;
        self.schedule_lease_renewal_v6(cmd_tx.clone(), iface.to_string(), lease);

        log!("network", "DHCPv6 acquired on {}: {}", iface, ipv6_cfg.address);

        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))
    }

    /// Acquire both IPv4 and IPv6 addresses (dual-stack)
    pub(super) async fn acquire_dual_stack(
        &mut self,
        iface: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<InterfaceSnapshot> {
        log!("network", "Acquiring dual-stack (IPv4 + IPv6) on {}", iface);

        let index = link::ensure_link_up(&self.handle, iface).await?;
        let mac = self.get_interface_mac(iface)?;

        // Run DHCPv4 and DHCPv6 in parallel for faster acquisition
        let dhcp4_future = run_dhcp_client(iface, &mac);
        let dhcp6_future = run_dhcpv6_client(iface, &mac);

        let (dhcp4_result, dhcp6_result) = tokio::join!(dhcp4_future, dhcp6_future);

        // Process IPv4 result
        match dhcp4_result {
            Ok((ip_cfg, lease)) => {
                self.apply_ip_configuration(index, &ip_cfg).await?;
                self.update_interface_with_lease(iface, ip_cfg.clone(), lease.clone())?;
                self.schedule_lease_renewal(cmd_tx.clone(), iface.to_string(), lease);
                log!("network", "DHCPv4 acquired on {}: {}", iface, ip_cfg.address);
            }
            Err(e) => {
                log!("network", "DHCPv4 failed on {}: {} (continuing with IPv6 only)", iface, e);
            }
        }

        // Process IPv6 result
        match dhcp6_result {
            Ok((ipv6_cfg, lease)) => {
                self.apply_ipv6_configuration(index, &ipv6_cfg).await?;
                self.update_interface_with_ipv6_lease(iface, ipv6_cfg.clone(), lease.clone())?;
                self.schedule_lease_renewal_v6(cmd_tx.clone(), iface.to_string(), lease);
                log!("network", "DHCPv6 acquired on {}: {}", iface, ipv6_cfg.address);
            }
            Err(e) => {
                log!("network", "DHCPv6 failed on {}: {} (continuing with IPv4 only)", iface, e);
            }
        }

        // Return the interface snapshot (will have whichever addresses succeeded)
        self.get_interface(iface)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("interface disappeared"))
    }

    async fn apply_ipv6_configuration(
        &mut self,
        index: u32,
        ipv6_cfg: &Ipv6Config,
    ) -> Result<()> {
        address::ensure_ipv6(&self.handle, index, ipv6_cfg.address, ipv6_cfg.prefix_len).await?;

        if let Some(gw) = ipv6_cfg.gateway {
            route::ensure_default_route_v6(&self.handle, gw).await?;
        }

        if !ipv6_cfg.dns.is_empty() {
            configure_dns_v6(&ipv6_cfg.dns)?;
        }

        Ok(())
    }

    fn update_interface_with_ipv6_lease(
        &mut self,
        iface: &str,
        ipv6_cfg: Ipv6Config,
        lease: DhcpLease,
    ) -> Result<()> {
        let iface_snap = self
            .get_interface_mut(iface)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface))?;

        iface_snap.ipv6 = Some(ipv6_cfg);
        iface_snap.ipv6_lease = Some(lease);
        self.sync_and_publish();

        Ok(())
    }

    fn schedule_lease_renewal_v6(
        &mut self,
        cmd_tx: mpsc::Sender<NetworkCommand>,
        iface: String,
        lease: DhcpLease,
    ) {
        let renew_deadline = lease.obtained_at + lease.renewal_time;
        let rebind_deadline = lease.obtained_at + lease.rebind_time;
        let expiry_deadline = lease.expiry();

        let renew_task = Self::spawn_renewal_task_v6(
            cmd_tx.clone(),
            iface.clone(),
            renew_deadline,
            "renewal",
        );
        let rebind_task = Self::spawn_renewal_task_v6(
            cmd_tx.clone(),
            iface.clone(),
            rebind_deadline,
            "rebind",
        );
        let expiry_task = Self::spawn_renewal_task_v6(
            cmd_tx,
            iface.clone(),
            expiry_deadline,
            "expiry",
        );

        // Track IPv6 renewal tasks with a v6 suffix to differentiate
        self.track_renewal_task(format!("{}:v6", iface), renew_task);
        self.track_renewal_task(format!("{}:v6", iface), rebind_task);
        self.track_renewal_task(format!("{}:v6", iface), expiry_task);
    }

    fn spawn_renewal_task_v6(
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

            log!("network", "DHCPv6 lease {} attempt for {}", task_name, iface);
            let _ = cmd_tx.send(NetworkCommand::RenewLeaseV6 { iface }).await;
        })
    }

    pub(super) async fn renew_lease_v6(&mut self, iface: &str) -> Result<()> {
        log!("network", "Renewing DHCPv6 lease for {}", iface);

        let mac = self
            .get_interface(iface)
            .map(|i| i.mac)
            .ok_or_else(|| anyhow::anyhow!("interface not tracked: {}", iface))?;

        match run_dhcpv6_client(iface, &mac).await {
            Ok((ipv6_cfg, lease)) => {
                let index = self
                    .get_interface(iface)
                    .ok_or_else(|| anyhow::anyhow!("interface disappeared"))?
                    .index;

                self.apply_ipv6_configuration(index, &ipv6_cfg).await?;
                self.update_interface_with_ipv6_lease(iface, ipv6_cfg, lease)?;

                log!("network", "DHCPv6 lease renewed for {}", iface);
                Ok(())
            }
            Err(e) => {
                log!("network", "DHCPv6 renewal failed for {}: {}", iface, e);
                Err(anyhow::anyhow!("DHCPv6 renewal failed: {}", e))
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
        let ipv4 = self.get_interface(primary).and_then(|i| i.ipv4.clone());
        let ipv6 = self.get_interface(primary).and_then(|i| i.ipv6.clone());

        let br_snapshot = InterfaceSnapshot {
            name: LAN_BRIDGE_NAME.to_string(),
            index,
            mac,
            link: LinkStateKind::Up,
            ipv4,
            ipv4_lease: Some(lease),
            ipv6,
            ipv6_lease: None,  // IPv6 lease transferred separately if exists
            #[allow(deprecated)]
            ip: self.get_interface(primary).and_then(|i| i.ipv4.clone()),
            #[allow(deprecated)]
            lease: Some(DhcpLease { obtained_at: std::time::SystemTime::now(), lease_time: std::time::Duration::from_secs(3600), renewal_time: std::time::Duration::from_secs(1800), rebind_time: std::time::Duration::from_secs(3150) }),
        };

        self.insert_interface(br_snapshot);
    }

    fn clear_lease_from_primary(&mut self, primary: &str) {
        if let Some(iface) = self.get_interface_mut(primary) {
            iface.ipv4 = None;
            iface.ipv4_lease = None;
            iface.ipv6 = None;
            iface.ipv6_lease = None;
            #[allow(deprecated)]
            {
                iface.ip = None;
                iface.lease = None;
            }
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
            ipv4: None,
            ipv4_lease: None,
            ipv6: None,
            ipv6_lease: None,
            #[allow(deprecated)]
            ip: None,
            #[allow(deprecated)]
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

    pub(super) async fn promote_secondary(
        &mut self,
        secondary_name: &str,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        log!("network", "Promoting secondary {} to active interface", secondary_name);

        let old_active = self.state.active.clone();
        
        // Update active interface (primary stays the same - it's our preference)
        self.state.active = Some(secondary_name.to_string());
        self.state.secondaries.retain(|n| n != secondary_name);
        
        // Add old active back to secondaries if it exists and isn't the primary
        if let Some(old) = &old_active {
            if old != secondary_name && !self.state.secondaries.contains(old) {
                self.state.secondaries.push(old.clone());
            }
            self.cancel_renewal_tasks(old);
            log!("network", "Cancelled renewal tasks for previous active: {}", old);
        }

        match self.acquire_dhcp(secondary_name, cmd_tx).await {
            Ok(_) => {
                log!(
                    "network",
                    "DHCP acquired on new active {}, migrating bridge",
                    secondary_name
                );

                if let Some(old) = old_active {
                    if self.migrate_bridge_uplink(&old, secondary_name).await.is_ok() {
                        log!(
                            "network",
                            "Failover complete: {} is now active with bridge migrated",
                            secondary_name
                        );
                        // If active != primary, we're in Degraded state
                        self.state.state = if self.state.is_on_primary() {
                            NetworkStateKind::Ready
                        } else {
                            NetworkStateKind::Degraded
                        };
                    } else {
                        log!("network", "Bridge migration failed, staying in degraded state");
                        self.state.state = NetworkStateKind::Degraded;
                    }
                } else {
                    // If active != primary, we're in Degraded state
                    self.state.state = if self.state.is_on_primary() {
                        NetworkStateKind::Ready
                    } else {
                        NetworkStateKind::Degraded
                    };
                }

                self.sync_and_publish();
                Ok(())
            }
            Err(e) => {
                log!(
                    "network",
                    "Failed to acquire DHCP on new active {}: {}",
                    secondary_name,
                    e
                );
                self.state.state = NetworkStateKind::Degraded;
                self.publish_state();
                Err(e)
            }
        }
    }

    async fn migrate_bridge_uplink(&mut self, old_iface: &str, new_iface: &str) -> Result<()> {
        log!(
            "network",
            "Migrating bridge from {} to {}",
            old_iface,
            new_iface
        );

        bridge::migrate_bridge_to_interface(&self.handle, LAN_BRIDGE_NAME, old_iface, new_iface)
            .await?;

        log!(
            "network",
            "Bridge successfully migrated to active interface: {}",
            new_iface
        );
        Ok(())
    }

    pub(super) async fn recover_primary(
        &mut self,
        from_secondary: &str,
        to_primary: &str,
    ) -> Result<()> {
        log!(
            "network",
            "Recovery: migrating bridge from secondary {} back to primary {}",
            from_secondary,
            to_primary
        );

        match self.migrate_bridge_uplink(from_secondary, to_primary).await {
            Ok(_) => {
                log!("network", "Primary {} recovered successfully", to_primary);
                
                // Update active to point back to primary
                self.state.active = Some(to_primary.to_string());
                
                // Move the previous active back to secondaries
                if from_secondary != to_primary && !self.state.secondaries.contains(&from_secondary.to_string()) {
                    self.state.secondaries.push(from_secondary.to_string());
                }
                self.state.secondaries.retain(|n| n != to_primary);
                
                // Back on primary = Ready state
                self.state.state = NetworkStateKind::Ready;
                self.publish_state();
                Ok(())
            }
            Err(e) => {
                log!(
                    "network",
                    "Failed to recover primary {}: {}",
                    to_primary,
                    e
                );
                Err(e)
            }
        }
    }
}
