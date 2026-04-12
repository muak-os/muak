//! Bridge provisioning logic for the network actor.

use anyhow::{Context, Result};
use netlib::bridge;
use netlib::interface::InterfaceName;
use netlib::link::LinkStateKind;
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::{InterfaceSnapshot, InterfaceState, NetworkActor};
use crate::dhcp::DhcpState;

impl NetworkActor {
    /// Resolves the physical port name for a bridge from its config.
    fn resolve_bridge_port<'a>(&self, ports: &'a [String], primary: &'a InterfaceName) -> &'a str {
        if ports.len() > 1 {
            kmsg::warn!(
                "bridge.port has {} entries; only the first is used (multi-port bridges not yet supported)",
                ports.len()
            );
        }

        match ports.first() {
            Some(p) if p == "auto" => primary.as_str(),
            Some(p) => p.as_str(),
            None => primary.as_str(),
        }
    }

    /// Creates or updates the bridge, transfers the IP from the physical port.
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
        let br_iface_name = InterfaceName::new(bridge_name)
            .with_context(|| format!("invalid bridge name: {bridge_name}"))?;
        let br_snapshot = InterfaceSnapshot {
            name: br_iface_name.clone(),
            state: InterfaceState::Configured,
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease.clone()),
            dhcp_state: Some(DhcpState::Bound),
            ipv6: None,
        };
        self.insert_interface(br_snapshot);

        if let Some(port_iface) = self.get_interface_mut(port_name) {
            port_iface.ip = None;
            port_iface.lease = None;
            port_iface.dhcp_state = None;
        }
        self.deconfigure_port(port_name);
        self.sync_and_publish();

        kmsg::info!(
            "Transferring DHCP lease management from {} to {}",
            port_name,
            bridge_name
        );
        self.schedule_lease_renewal(cmd_tx.clone(), br_iface_name, &lease);

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

    /// Walks a port interface through `Configured -> Deconfiguring -> Discovered` after IP teardown.
    fn deconfigure_port(&mut self, port_name: &str) {
        self.set_interface_state(port_name, InterfaceState::Deconfiguring);
        self.set_interface_state(port_name, InterfaceState::Discovered);
    }
}
