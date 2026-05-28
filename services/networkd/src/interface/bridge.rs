//! Bridge provisioning logic for a per-interface actor.

use anyhow::{Context, Result};
use netlib::interface::Name;
use netlib::link::State;
use netlib::netlink::Ops;

use super::InterfaceActor;
use crate::dhcp::DhcpState;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::state::InterfaceState;

impl<N: Ops> InterfaceActor<N> {
    /// Creates the bridge device, transfers the IP from this port, and returns the bridge snapshot.
    pub(super) async fn configure_bridge(
        &mut self,
        bridge_name: &str,
        stp: bool,
    ) -> Result<InterfaceSnapshot> {
        let lease = self
            .snapshot
            .lease
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", self.snapshot.name))?;
        let mac = self.snapshot.mac;
        let gateway = self.snapshot.ip.as_ref().and_then(|ip| ip.gateway);

        kmsg::info!(
            "Setting up bridge {} with port {}",
            bridge_name,
            self.snapshot.name
        );
        self.ops
            .ensure_bridge(bridge_name, self.snapshot.name.as_str(), gateway, stp)
            .await?;
        kmsg::info!(
            "Bridge setup complete: {} <- {}",
            bridge_name,
            self.snapshot.name
        );

        self.timers.disarm();

        let index = self.ops.index(bridge_name).await?;
        let ip = self.snapshot.ip.clone();
        let br_iface_name = Name::new(bridge_name)
            .with_context(|| format!("invalid bridge name: {bridge_name}"))?;

        let bridge_snapshot = InterfaceSnapshot {
            name: br_iface_name,
            state: InterfaceState::Configured,
            index,
            mac,
            link: State::Up,
            ip,
            lease: Some(lease),
            dhcp_state: Some(DhcpState::Bound),
            ipv6: None,
            l3_owner: Name::new(bridge_name)
                .with_context(|| format!("invalid bridge name: {bridge_name}"))?,
        };

        let snapshot = &mut self.snapshot;
        snapshot.ip = None;
        snapshot.lease = None;
        snapshot.dhcp_state = None;
        snapshot.l3_owner = bridge_snapshot.name.clone();
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
