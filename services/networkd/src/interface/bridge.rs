//! Bridge provisioning logic for a per-interface actor.

use anyhow::{Context, Result};
use netlib::bridge;
use netlib::interface::InterfaceName;
use netlib::link::LinkStateKind;

use super::InterfaceActor;
use crate::dhcp::DhcpState;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::state::InterfaceState;

impl InterfaceActor {
    /// Creates the bridge device, transfers the IP from this port, and returns the bridge snapshot.
    pub(super) async fn configure_bridge(
        &mut self,
        bridge_name: &str,
        stp: bool,
    ) -> Result<InterfaceSnapshot> {
        let port_name = self.snapshot.name.to_string();
        let lease = self
            .snapshot
            .lease
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", port_name))?;
        let mac = self.snapshot.mac;
        let gateway = self.snapshot.ip.as_ref().and_then(|ip| ip.gateway);

        kmsg::info!("Setting up bridge {} with port {}", bridge_name, port_name);
        bridge::ensure_with_config(&self.handle, bridge_name, &port_name, gateway, stp).await?;
        kmsg::info!("Bridge setup complete: {} <- {}", bridge_name, port_name);

        self.cancel_renewal_tasks();

        let index = netlib::link::get_index(&self.handle, bridge_name).await?;
        let ip = self.snapshot.ip.clone();
        let br_iface_name = InterfaceName::new(bridge_name)
            .with_context(|| format!("invalid bridge name: {bridge_name}"))?;

        let bridge_snapshot = InterfaceSnapshot {
            name: br_iface_name,
            state: InterfaceState::Configured,
            index,
            mac,
            link: LinkStateKind::Up,
            ip,
            lease: Some(lease),
            dhcp_state: Some(DhcpState::Bound),
            ipv6: None,
        };

        self.snapshot.ip = None;
        self.snapshot.lease = None;
        self.snapshot.dhcp_state = None;
        self.deconfigure();
        self.publish_snapshot();

        Ok(bridge_snapshot)
    }

    /// Walks this interface through `Configured -> Deconfiguring -> Discovered`.
    fn deconfigure(&mut self) {
        self.set_state(InterfaceState::Deconfiguring);
        self.set_state(InterfaceState::Discovered);
    }
}
