//! Periodic reconciliation of configured network intent.

use anyhow::Result;
use config::{BridgeConfig, InterfaceConfig, InterfaceKind};
use netlib::interface::InterfaceName;
use netlib::ops::NetlinkOps;

use super::NetworkSupervisor;
use crate::interface::ApplyMode;
use crate::interface::InterfaceCommand;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::state::InterfaceState;
use crate::supervisor::state::NetworkState;

/// Describes whether reconcile applied intent or skipped it deliberately.
enum ReconcileDisposition {
    Applied,
    Skipped(String),
}

impl<N: NetlinkOps> NetworkSupervisor<N> {
    /// Reapplies declarative interface configuration to converge drifted state.
    pub(super) async fn reconcile(&mut self) {
        if !self.should_reconcile() {
            return;
        }

        let interfaces = self.config.interfaces.clone();
        for iface_cfg in &interfaces {
            self.try_reconcile_interface(iface_cfg).await;
        }

        self.sync_and_publish();
    }

    /// Returns whether the supervisor is ready to reconcile runtime state.
    fn should_reconcile(&self) -> bool {
        matches!(
            self.state.state,
            NetworkState::Operational | NetworkState::Ready
        )
    }

    /// Reapplies one configured interface intent.
    async fn try_reconcile_interface(&mut self, iface_cfg: &InterfaceConfig) {
        match self.reconcile_interface(iface_cfg).await {
            Ok(ReconcileDisposition::Applied) => {}
            Ok(ReconcileDisposition::Skipped(reason)) => {
                println!("Skipping reconcile for {}: {}", iface_cfg.name, reason);
            }
            Err(e) => {
                kmsg::warn!("Failed to reconcile {}: {}", iface_cfg.name, e);
            }
        }
    }

    /// Reapplies one configured interface intent from the declarative config.
    async fn reconcile_interface(
        &mut self,
        iface_cfg: &InterfaceConfig,
    ) -> Result<ReconcileDisposition> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => self.reconcile_bridge(iface_cfg).await,
            InterfaceKind::Ethernet => self.reconcile_ethernet(iface_cfg).await,
        }
    }

    /// Reapplies an Ethernet interface intent when the port still owns the configuration.
    async fn reconcile_ethernet(
        &mut self,
        iface_cfg: &InterfaceConfig,
    ) -> Result<ReconcileDisposition> {
        let iface_name = self.resolve_interface_name(&iface_cfg.name)?;
        let Some(actor_handle) = self.interfaces.get(&iface_name) else {
            return Ok(ReconcileDisposition::Skipped(format!(
                "interface '{}' is not known at runtime",
                iface_name
            )));
        };

        if self.is_bridge_owned(actor_handle) {
            return Ok(ReconcileDisposition::Skipped(format!(
                "interface '{}' is bridge-owned by {}",
                iface_name,
                actor_handle.state_rx.borrow().l3_owner
            )));
        }

        println!("Reconciling ethernet interface: {}", iface_name);
        let index = self.ops.ensure_link_up(iface_name.as_str()).await?;

        if let Some(ipv4) = iface_cfg.ipv4.as_ref() {
            self.reconcile_ipv4(&iface_name, index, ipv4).await?;
        }

        if let Some(ipv6) = iface_cfg.ipv6.as_ref() {
            self.reconcile_ipv6(&iface_name, index, ipv6).await?;
        }

        Ok(ReconcileDisposition::Applied)
    }

    /// Reapplies a bridge intent and refreshes the bridge actor when needed.
    async fn reconcile_bridge(
        &mut self,
        iface_cfg: &InterfaceConfig,
    ) -> Result<ReconcileDisposition> {
        let bridge_name = InterfaceName::new(&iface_cfg.name)?;
        let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
        if self.interfaces.contains_key(&bridge_name) {
            kmsg::info!("Reconciling bridge interface: {}", bridge_name);
            let bridge_snapshot = self
                .reconcile_bridge_snapshot(&bridge_name, &bridge_cfg)
                .await?;
            self.respawn_interface_actor(bridge_snapshot).await;

            return Ok(ReconcileDisposition::Applied);
        }

        let Some(port_iface_name) = self.ready_bridge_port(&bridge_cfg)? else {
            return Ok(ReconcileDisposition::Skipped(
                "bridge port is not configured with a lease".to_string(),
            ));
        };

        println!(
            "Reconciling bridge interface: {} via port {}",
            bridge_name, port_iface_name
        );
        self.provision_bridge(bridge_name.as_str(), &bridge_cfg)
            .await?;

        Ok(ReconcileDisposition::Applied)
    }

    /// Reapplies bridge configuration and returns the refreshed bridge snapshot.
    async fn reconcile_bridge_snapshot(
        &mut self,
        bridge_name: &InterfaceName,
        bridge_cfg: &BridgeConfig,
    ) -> Result<InterfaceSnapshot> {
        let bridge_handle = self
            .interfaces
            .get(bridge_name)
            .ok_or_else(|| anyhow::anyhow!("bridge interface '{}' not found", bridge_name))?;
        let bridge_snapshot = bridge_handle.state_rx.borrow().clone();
        let (port_iface_name, _) = self.bridge_port_handle(bridge_cfg)?;
        let gateway = bridge_snapshot.ip.as_ref().and_then(|ip| ip.gateway);
        self.ops
            .ensure_bridge(
                bridge_name.as_str(),
                port_iface_name.as_str(),
                gateway,
                bridge_cfg.stp,
            )
            .await?;

        let index = self.ops.get_link_index(bridge_name.as_str()).await?;
        Ok(InterfaceSnapshot {
            name: bridge_name.clone(),
            state: InterfaceState::Configured,
            index,
            mac: bridge_snapshot.mac,
            link: netlib::link::LinkStateKind::Up,
            ip: bridge_snapshot.ip.clone(),
            lease: bridge_snapshot.lease.clone(),
            dhcp_state: bridge_snapshot.dhcp_state.clone(),
            ipv6: bridge_snapshot.ipv6.clone(),
            l3_owner: bridge_name.clone(),
        })
    }

    /// Reapplies IPv4 configuration for one interface.
    async fn reconcile_ipv4(
        &self,
        iface_name: &netlib::interface::InterfaceName,
        index: u32,
        ipv4: &config::Ipv4InterfaceConfig,
    ) -> Result<()> {
        if ipv4.dhcp {
            println!("Reconciling DHCP on {}", iface_name);
        } else if !ipv4.addresses.is_empty() {
            println!("Reconciling static IPv4 on {}", iface_name);
        }

        let actor_handle = self
            .interfaces
            .get(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface actor not found: {}", iface_name))?;

        if ipv4.dhcp {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureDhcp {
                    mode: ApplyMode::Reconcile,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        } else if !ipv4.addresses.is_empty() {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureStaticIpv4 {
                    mode: ApplyMode::Reconcile,
                    index,
                    addresses: ipv4.addresses.clone(),
                    gateway: ipv4.gateway,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        }

        Ok(())
    }

    /// Reapplies IPv6 configuration for one interface.
    async fn reconcile_ipv6(
        &self,
        iface_name: &netlib::interface::InterfaceName,
        index: u32,
        ipv6: &config::Ipv6InterfaceConfig,
    ) -> Result<()> {
        if !ipv6.addresses.is_empty() {
            println!("Reconciling static IPv6 on {}", iface_name);
        } else if ipv6.autoconf && self.config.ipv6 {
            println!("Reconciling SLAAC on {}", iface_name);
        }

        let actor_handle = self
            .interfaces
            .get(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface actor not found: {}", iface_name))?;

        if !ipv6.addresses.is_empty() {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureStaticIpv6 {
                    mode: ApplyMode::Reconcile,
                    index,
                    addresses: ipv6.addresses.clone(),
                    gateway: ipv6.gateway,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        } else if ipv6.autoconf && self.config.ipv6 {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureSlaac {
                    mode: ApplyMode::Reconcile,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        }

        Ok(())
    }

    /// Returns true when a bridge actor already owns the port actor's lease and address state.
    fn is_bridge_owned(&self, handle: &crate::interface::InterfaceActorHandle) -> bool {
        let snap = handle.state_rx.borrow();
        snap.l3_owner != snap.name
    }

    /// Returns the bridge port name when the port is configured enough to reconcile the bridge.
    fn ready_bridge_port(&self, bridge_cfg: &BridgeConfig) -> Result<Option<InterfaceName>> {
        let (port_iface_name, state_rx) = match self.bridge_port_handle(bridge_cfg) {
            Ok(port) => port,
            Err(_) => return Ok(None),
        };
        let snap = state_rx.borrow();
        if snap.state == InterfaceState::Configured && snap.lease.is_some() {
            return Ok(Some(port_iface_name));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::time::Duration;

    use netlib::address::IpConfig;
    use netlib::interface::InterfaceName;
    use netlib::link::LinkOps;
    use netlib::link::LinkStateKind;
    use tokio::sync::watch;

    use super::*;
    use crate::dhcp::{DhcpLease, DhcpState};
    use crate::dns::DnsState;
    use crate::interface::snapshot::InterfaceSnapshot;
    mod fixtures_netlink {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/netlink.rs"
        ));
    }

    use fixtures_netlink::MockNetlinkOps;

    fn master_index(mock: &MockNetlinkOps, name: &str) -> Option<u32> {
        mock.state
            .lock()
            .expect("lock")
            .links
            .get(name)
            .and_then(|link| link.master_index)
    }

    /// Returns a config with a single managed bridge.
    fn bridge_config() -> Arc<config::NetworkConfig> {
        let mut cfg = config::NetworkConfig::default();
        cfg.interfaces = vec![config::InterfaceConfig {
            name: "br0".to_string(),
            kind: InterfaceKind::Bridge,
            ipv4: None,
            ipv6: None,
            bridge: Some(BridgeConfig {
                port: vec!["eth0".to_string()],
                stp: false,
            }),
        }];
        Arc::new(cfg)
    }

    /// Returns a bridge-owned snapshot with cached lease state.
    fn bridge_snapshot(index: u32) -> InterfaceSnapshot {
        InterfaceSnapshot {
            name: InterfaceName::new("br0").expect("valid bridge name"),
            state: InterfaceState::Configured,
            index,
            mac: [0xAA; 6],
            link: LinkStateKind::Up,
            ip: Some(IpConfig {
                address: Ipv4Addr::new(192, 168, 10, 2),
                prefix_len: 24,
                gateway: Some(Ipv4Addr::new(192, 168, 10, 1)),
                dns: vec![],
            }),
            lease: Some(DhcpLease {
                obtained_at: std::time::SystemTime::now(),
                lease_time: Duration::from_secs(3600),
                renewal_time: Duration::from_secs(1800),
                rebind_time: Duration::from_secs(3150),
                server_ip: Ipv4Addr::new(192, 168, 10, 1),
                assigned_ip: Ipv4Addr::new(192, 168, 10, 2),
                prefix_len: 24,
                gateway: Some(Ipv4Addr::new(192, 168, 10, 1)),
                dns_servers: vec![],
            }),
            dhcp_state: Some(DhcpState::Bound),
            ipv6: None,
            l3_owner: InterfaceName::new("br0").expect("valid bridge owner"),
        }
    }

    /// Returns a port snapshot already owned by the bridge.
    fn port_snapshot(index: u32) -> InterfaceSnapshot {
        InterfaceSnapshot {
            name: InterfaceName::new("eth0").expect("valid port name"),
            state: InterfaceState::Discovered,
            index,
            mac: [0xAA; 6],
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
            l3_owner: InterfaceName::new("br0").expect("valid bridge owner"),
        }
    }

    #[tokio::test]
    async fn reconcile_bridge_refreshes_bridge_actor_state() {
        // ARRANGE
        let ops = MockNetlinkOps::new();
        let port_index = ops.add_link("eth0", [0xAA; 6], true);
        let bridge_index = ops.add_link("br0", [0xBB; 6], true);
        let (watch_tx, _) = watch::channel(crate::supervisor::snapshot::NetworkSnapshot::empty());
        let mut supervisor =
            NetworkSupervisor::new(ops.clone(), bridge_config(), watch_tx, DnsState::default());
        supervisor.state.state = NetworkState::Ready;
        supervisor.state.primary = Some(InterfaceName::new("eth0").expect("valid primary"));
        supervisor.spawn_interface_actor(port_snapshot(port_index));
        supervisor.spawn_interface_actor(bridge_snapshot(bridge_index));
        ops.delete_link(bridge_index)
            .await
            .expect("delete bridge link");

        // ACT
        let disposition = supervisor
            .reconcile_interface(&bridge_config().interfaces[0])
            .await
            .expect("bridge reconcile should succeed");

        // ASSERT
        assert!(matches!(disposition, ReconcileDisposition::Applied));
        let new_bridge_index = ops
            .get_link_index("br0")
            .await
            .expect("bridge should be recreated");
        assert_ne!(new_bridge_index, bridge_index, "bridge should be refreshed");
        assert_eq!(
            master_index(&ops, "eth0"),
            Some(new_bridge_index),
            "port should remain attached to the refreshed bridge"
        );
    }
}
