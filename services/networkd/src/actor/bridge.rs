//! Bridge provisioning logic for the network actor.

use anyhow::Result;
use netlib::bridge;
use netlib::link::LinkStateKind;
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::{InterfaceSnapshot, NetworkActor};

impl NetworkActor {
    /// Resolves the physical port name for a bridge from its config.
    fn resolve_bridge_port<'a>(&self, ports: &'a [String], primary: &'a str) -> &'a str {
        if ports.len() > 1 {
            kmsg::warn!(
                "bridge.port has {} entries; only the first is used (multi-port bridges not yet supported)",
                ports.len()
            );
        }

        match ports.first() {
            Some(p) if p == "auto" => primary,
            Some(p) => p.as_str(),
            None => primary,
        }
    }

    /// Creates or updates the bridge, transfers the IP from the physical port,
    pub(super) async fn configure_bridge(
        &mut self,
        bridge_name: &str,
        port_name: &str,
        stp: bool,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let (lease, mac, gateway) = self.extract_lease_mac_and_gateway(port_name)?;

        kmsg::info!("Setting up bridge {} with port {}", bridge_name, port_name);
        bridge::ensure_with_config(&self.handle, bridge_name, port_name, gateway, stp).await?;
        kmsg::info!("Bridge setup complete: {} <- {}", bridge_name, port_name);

        self.cancel_renewal_tasks(port_name);

        let index = netlib::link::get_index(&self.handle, bridge_name).await?;
        let ip = self.get_interface(port_name).and_then(|i| i.ip.clone());
        let br_snapshot = InterfaceSnapshot {
            name: bridge_name.to_string(),
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease.clone()),
            ipv6: None,
        };
        self.insert_interface(br_snapshot);

        if let Some(port_iface) = self.get_interface_mut(port_name) {
            port_iface.ip = None;
            port_iface.lease = None;
        }
        self.sync_and_publish();

        kmsg::info!(
            "Transferring DHCP lease management from {} to {}",
            port_name,
            bridge_name
        );
        self.schedule_lease_renewal(cmd_tx.clone(), bridge_name.to_string(), lease);

        Ok(())
    }

    pub(super) async fn setup_bridge_from_config(
        &mut self,
        bridge_name: &str,
        bridge_cfg: &config::BridgeConfig,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let port_name = self
            .resolve_bridge_port(&bridge_cfg.port, &primary)
            .to_string();

        self.configure_bridge(bridge_name, &port_name, bridge_cfg.stp, cmd_tx)
            .await
    }

    /// gRPC `SetupBridge` RPC handler — uses the configured bridge name and auto-resolves the port.
    pub(super) async fn setup_bridge(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;

        let bridge_name = self
            .bridge_name()
            .ok_or_else(|| anyhow::anyhow!("no bridge interface configured"))?
            .to_string();

        let bridge_cfg = config::network()
            .interfaces
            .iter()
            .find(|i| i.name == bridge_name)
            .and_then(|i| i.bridge.clone())
            .unwrap_or_default();

        let port_name = self
            .resolve_bridge_port(&bridge_cfg.port, &primary)
            .to_string();

        self.configure_bridge(&bridge_name, &port_name, bridge_cfg.stp, cmd_tx)
            .await
    }
}
