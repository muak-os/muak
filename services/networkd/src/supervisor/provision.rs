//! Applies declarative interface configuration from the config file via interface actors.

use anyhow::Result;
use config::{InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig};
use netlib::interface::InterfaceName;
use netlib::link;
use tokio::sync::oneshot;

use super::NetworkSupervisor;
use crate::interface::InterfaceCommand;

impl NetworkSupervisor {
    pub(super) async fn apply_interface_configs(&mut self) -> Result<()> {
        let interfaces = config::network().interfaces.clone();
        for iface_cfg in &interfaces {
            self.setup_interface_from_config(iface_cfg).await?;
        }
        Ok(())
    }

    async fn setup_interface_from_config(
        &mut self,
        iface_cfg: &config::InterfaceConfig,
    ) -> Result<()> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => {
                let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
                self.setup_bridge_from_config(&iface_cfg.name, &bridge_cfg)
                    .await?;
            }
            InterfaceKind::Ethernet => {
                self.setup_ethernet_from_config(
                    &iface_cfg.name,
                    iface_cfg.ipv4.as_ref(),
                    iface_cfg.ipv6.as_ref(),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn setup_ethernet_from_config(
        &mut self,
        name: &str,
        ipv4_cfg: Option<&Ipv4InterfaceConfig>,
        ipv6_cfg: Option<&Ipv6InterfaceConfig>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let iface_name = if name == "auto" {
            primary.clone()
        } else {
            InterfaceName::new(name)?
        };

        let actor_handle = self
            .interfaces
            .get(&iface_name)
            .ok_or_else(|| anyhow::anyhow!("ethernet interface '{}' not found", iface_name))?;

        kmsg::info!("Configuring ethernet interface: {}", iface_name);
        let index = link::ensure_up(&self.handle, iface_name.as_str()).await?;

        match ipv4_cfg {
            Some(ipv4) if ipv4.dhcp => {
                let (reply_tx, reply_rx) = oneshot::channel();
                actor_handle
                    .cmd_tx
                    .send(InterfaceCommand::ConfigureDhcp { reply: reply_tx })
                    .await
                    .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
                reply_rx.await??;
            }
            Some(ipv4) if !ipv4.addresses.is_empty() => {
                actor_handle
                    .cmd_tx
                    .send(InterfaceCommand::ConfigureStaticIpv4 {
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
            self.configure_ipv6_for_interface(&iface_name, index, ipv6)
                .await?;
        }

        kmsg::info!("Ethernet interface configured: {}", iface_name);
        Ok(())
    }

    async fn configure_ipv6_for_interface(
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
                    index,
                    addresses: ipv6.addresses.clone(),
                    gateway: ipv6.gateway,
                })
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        } else if ipv6.autoconf && config::network().ipv6 {
            actor_handle
                .cmd_tx
                .send(InterfaceCommand::ConfigureSlaac)
                .await
                .map_err(|_| anyhow::anyhow!("interface actor gone: {}", iface_name))?;
        }
        Ok(())
    }

    async fn setup_bridge_from_config(
        &mut self,
        bridge_name: &str,
        bridge_cfg: &config::BridgeConfig,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let port_name = resolve_bridge_port(&bridge_cfg.port, &primary);

        let port_iface_name = InterfaceName::new(&port_name)?;
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

fn resolve_bridge_port(ports: &[String], primary: &InterfaceName) -> String {
    if ports.len() > 1 {
        kmsg::warn!(
            "bridge.port has {} entries; only the first is used \
             (multi-port bridges not yet supported)",
            ports.len()
        );
    }

    match ports.first() {
        Some(p) if p == "auto" => primary.to_string(),
        Some(p) => p.clone(),
        None => primary.to_string(),
    }
}
