//! Applies declarative interface configuration from the config file via interface actors.

use std::borrow::Cow;

use anyhow::Result;
use config::{
    BridgeConfig, InterfaceConfig, InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig,
};
use netlib::interface::InterfaceName;
use netlib::ops::NetlinkOps;
use tokio::sync::oneshot;

use super::NetworkSupervisor;
use crate::interface::ApplyMode;
use crate::interface::InterfaceCommand;
use crate::interface::snapshot::InterfaceSnapshot;
use crate::interface::state::InterfaceState;

impl<N: NetlinkOps> NetworkSupervisor<N> {
    pub(super) async fn provision_interfaces(&mut self) {
        let interfaces = self.config.interfaces.clone();
        for iface_cfg in &interfaces {
            self.try_provision_interface(iface_cfg).await;
        }
    }

    async fn try_provision_interface(&mut self, iface_cfg: &InterfaceConfig) {
        if let Err(e) = self.provision_interface_from_config(iface_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }

    async fn provision_interface_from_config(&mut self, iface_cfg: &InterfaceConfig) -> Result<()> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => {
                let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
                self.provision_bridge(&iface_cfg.name, &bridge_cfg).await?;
            }
            InterfaceKind::Ethernet => {
                self.provision_ethernet(
                    &iface_cfg.name,
                    iface_cfg.ipv4.as_ref(),
                    iface_cfg.ipv6.as_ref(),
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Resolves a configured interface name, expanding the `auto` alias.
    pub(super) fn resolve_interface_name(&self, name: &str) -> Result<InterfaceName> {
        let primary = self.get_primary_name()?;
        if name == "auto" {
            Ok(primary)
        } else {
            InterfaceName::new(name).map_err(Into::into)
        }
    }

    async fn provision_ethernet(
        &mut self,
        name: &str,
        ipv4_cfg: Option<&Ipv4InterfaceConfig>,
        ipv6_cfg: Option<&Ipv6InterfaceConfig>,
    ) -> Result<()> {
        let iface_name = self.resolve_interface_name(name)?;

        let actor_handle = self
            .interfaces
            .get(&iface_name)
            .ok_or_else(|| anyhow::anyhow!("ethernet interface '{}' not found", iface_name))?;

        kmsg::info!("Configuring ethernet interface: {}", iface_name);
        let index = self.ops.ensure_link_up(iface_name.as_str()).await?;

        match ipv4_cfg {
            Some(ipv4) if ipv4.dhcp => {
                actor_handle
                    .cmd_tx
                    .send(InterfaceCommand::ConfigureDhcp {
                        mode: ApplyMode::Provision,
                    })
                    .await
                    .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
            }
            Some(ipv4) if !ipv4.addresses.is_empty() => {
                actor_handle
                    .cmd_tx
                    .send(InterfaceCommand::ConfigureStaticIpv4 {
                        mode: ApplyMode::Provision,
                        index,
                        addresses: ipv4.addresses.clone(),
                        gateway: ipv4.gateway,
                    })
                    .await
                    .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
            }
            _ => {}
        }

        if let Some(ipv6) = ipv6_cfg {
            self.provision_ipv6(&iface_name, index, ipv6).await?;
        }

        Ok(())
    }

    async fn provision_ipv6(
        &self,
        iface_name: &InterfaceName,
        index: u32,
        ipv6: &Ipv6InterfaceConfig,
    ) -> Result<()> {
        let actor_handle = self
            .interfaces
            .get(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface actor not found: {}", iface_name))?;

        if !ipv6.addresses.is_empty() {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureStaticIpv6 {
                    mode: ApplyMode::Provision,
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
                    mode: ApplyMode::Provision,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        }

        Ok(())
    }

    pub(super) async fn provision_bridge(
        &mut self,
        bridge_name: &str,
        bridge_cfg: &BridgeConfig,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let port_name = resolve_bridge_port(&bridge_cfg.port, &primary);

        let port_iface_name = InterfaceName::new(&*port_name)?;
        let actor_handle = self
            .interfaces
            .get(&port_iface_name)
            .ok_or_else(|| anyhow::anyhow!("bridge port '{}' not found", port_name))?;

        let mut state_rx = actor_handle.state_rx.clone();
        wait_for_configured(&mut state_rx).await?;

        let actor_handle = self
            .interfaces
            .get(&port_iface_name)
            .ok_or_else(|| anyhow::anyhow!("bridge port '{}' not found", port_name))?;

        let (reply_tx, reply_rx) = oneshot::channel();
        actor_handle
            .cmd_tx
            .send(InterfaceCommand::ConfigureBridge {
                bridge_name: bridge_name.to_string(),
                stp: bridge_cfg.stp,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("interface actor gone: {}", port_name))?;

        let bridge_snapshot = reply_rx.await??;
        self.spawn_interface_actor(bridge_snapshot);

        Ok(())
    }
}

/// Waits until the interface actor reports `Configured`.
async fn wait_for_configured(
    rx: &mut tokio::sync::watch::Receiver<std::sync::Arc<InterfaceSnapshot>>,
) -> Result<()> {
    loop {
        if rx.borrow().state == InterfaceState::Configured {
            return Ok(());
        }
        rx.changed()
            .await
            .map_err(|_| anyhow::anyhow!("interface actor dropped before reaching Configured"))?;
    }
}

fn resolve_bridge_port<'a>(ports: &'a [String], primary: &'a InterfaceName) -> Cow<'a, str> {
    if ports.len() > 1 {
        kmsg::warn!(
            "bridge.port has {} entries; only the first is used \
             (multi-port bridges not yet supported)",
            ports.len()
        );
    }

    match ports.first() {
        Some(p) if p == "auto" => Cow::Borrowed(primary.as_str()),
        Some(p) => Cow::Borrowed(p.as_str()),
        None => Cow::Borrowed(primary.as_str()),
    }
}
