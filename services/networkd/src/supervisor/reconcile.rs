//! Periodic reconciliation of configured network intent.

use std::borrow::Cow;

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

impl<N: NetlinkOps> NetworkSupervisor<N> {
    /// Re-applies declarative interface configuration to converge drifted state.
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

    /// Re-applies one configured interface intent.
    async fn try_reconcile_interface(&mut self, iface_cfg: &InterfaceConfig) {
        if let Err(e) = self.reconcile_interface(iface_cfg).await {
            kmsg::warn!("Failed to reconcile {}: {}", iface_cfg.name, e);
        }
    }

    /// Re-applies one configured interface intent from the declarative config.
    async fn reconcile_interface(&mut self, iface_cfg: &InterfaceConfig) -> Result<()> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => self.reconcile_bridge(iface_cfg).await,
            InterfaceKind::Ethernet => self.reconcile_ethernet(iface_cfg).await,
        }
    }

    /// Re-applies an ethernet interface intent when the port still owns the configuration.
    async fn reconcile_ethernet(&mut self, iface_cfg: &InterfaceConfig) -> Result<()> {
        let iface_name = self.resolve_interface_name(&iface_cfg.name)?;
        let Some(actor_handle) = self.interfaces.get(&iface_name) else {
            return Err(anyhow::anyhow!(
                "ethernet interface '{}' not found",
                iface_name
            ));
        };

        if self.is_bridge_owned(actor_handle) {
            return Ok(());
        }

        let index = self.ops.ensure_link_up(iface_name.as_str()).await?;

        if let Some(ipv4) = iface_cfg.ipv4.as_ref() {
            self.reconcile_ipv4(&iface_name, index, ipv4).await?;
        }

        if let Some(ipv6) = iface_cfg.ipv6.as_ref() {
            self.reconcile_ipv6(&iface_name, index, ipv6).await?;
        }

        Ok(())
    }

    /// Re-applies a bridge intent only when the bridge actor is not already present.
    async fn reconcile_bridge(&mut self, iface_cfg: &InterfaceConfig) -> Result<()> {
        let bridge_name = InterfaceName::new(&iface_cfg.name)?;
        if self.interfaces.contains_key(&bridge_name) {
            return Ok(());
        }

        let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
        if !self.bridge_port_ready(&bridge_cfg)? {
            return Ok(());
        }

        self.provision_bridge(&iface_cfg.name, &bridge_cfg).await
    }

    /// Re-applies IPv4 configuration for one interface.
    async fn reconcile_ipv4(
        &self,
        iface_name: &netlib::interface::InterfaceName,
        index: u32,
        ipv4: &config::Ipv4InterfaceConfig,
    ) -> Result<()> {
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

    /// Re-applies IPv6 configuration for one interface.
    async fn reconcile_ipv6(
        &self,
        iface_name: &netlib::interface::InterfaceName,
        index: u32,
        ipv6: &config::Ipv6InterfaceConfig,
    ) -> Result<()> {
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
        self.is_detached_bridge_port(&snap) && self.bridge_owner_exists(&snap)
    }

    /// Returns true when the bridge port is configured enough to instantiate a bridge actor.
    fn bridge_port_ready(&self, bridge_cfg: &BridgeConfig) -> Result<bool> {
        let primary = self.get_primary_name()?;
        let port_name = resolve_bridge_port(&bridge_cfg.port, &primary);
        let port_iface_name = InterfaceName::new(&*port_name)?;
        let Some(actor_handle) = self.interfaces.get(&port_iface_name) else {
            return Ok(false);
        };

        let snap = actor_handle.state_rx.borrow();
        Ok(snap.state == InterfaceState::Configured && snap.lease.is_some())
    }

    /// Returns true when a port snapshot has been stripped after bridge transfer.
    fn is_detached_bridge_port(&self, snap: &InterfaceSnapshot) -> bool {
        snap.state == InterfaceState::Discovered
            && snap.ip.is_none()
            && snap.lease.is_none()
            && snap.dhcp_state.is_none()
    }

    /// Returns true when another actor appears to own a detached bridge port.
    fn bridge_owner_exists(&self, snap: &InterfaceSnapshot) -> bool {
        self.interfaces
            .values()
            .any(|other_handle| self.is_bridge_owner_candidate(snap, other_handle))
    }

    /// Returns true when a peer actor looks like the bridge owner for a detached port.
    fn is_bridge_owner_candidate(
        &self,
        snap: &InterfaceSnapshot,
        other_handle: &crate::interface::InterfaceActorHandle,
    ) -> bool {
        let other = other_handle.state_rx.borrow();
        other.name != snap.name
            && other.mac == snap.mac
            && other.state != InterfaceState::Discovered
    }
}

/// Resolves the effective bridge port name for reconciliation checks.
fn resolve_bridge_port<'a>(ports: &'a [String], primary: &'a InterfaceName) -> Cow<'a, str> {
    match ports.first() {
        Some(port) if port == "auto" => Cow::Borrowed(primary.as_str()),
        Some(port) => Cow::Borrowed(port.as_str()),
        None => Cow::Borrowed(primary.as_str()),
    }
}
