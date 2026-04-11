//! Applies declarative interface configuration from the config file to the network actor.

use anyhow::{Result, bail};
use config::{InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig};
use netlib::link;
use tokio::sync::mpsc;

use super::commands::NetworkCommand;
use super::state::NetworkActor;

impl NetworkActor {
    pub(super) async fn apply_interface_configs(
        &mut self,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let interfaces = config::network().interfaces.clone();
        for iface_cfg in &interfaces {
            self.setup_interface_from_config(iface_cfg, cmd_tx).await?;
        }
        Ok(())
    }

    async fn setup_interface_from_config(
        &mut self,
        iface_cfg: &config::InterfaceConfig,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        match iface_cfg.kind {
            InterfaceKind::Bridge => {
                let bridge_cfg = iface_cfg.bridge.as_ref().cloned().unwrap_or_default();
                self.setup_bridge_from_config(&iface_cfg.name, &bridge_cfg, cmd_tx)
                    .await?;
            }
            InterfaceKind::Ethernet => {
                self.setup_ethernet_from_config(
                    &iface_cfg.name,
                    iface_cfg.ipv4.as_ref(),
                    iface_cfg.ipv6.as_ref(),
                    cmd_tx,
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
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        let primary = self.get_primary_name()?;
        let iface_name = if name == "auto" {
            primary.as_str()
        } else {
            name
        };

        if !self.has_interface(iface_name) {
            bail!("ethernet interface '{}' not found", iface_name);
        }

        kmsg::info!("Configuring ethernet interface: {}", iface_name);
        let index = link::ensure_up(&self.handle, iface_name).await?;

        match ipv4_cfg {
            Some(ipv4) if ipv4.dhcp => {
                self.acquire_dhcp(iface_name, cmd_tx).await?;
            }
            Some(ipv4) if !ipv4.addresses.is_empty() => {
                self.apply_static_ipv4(iface_name, index, &ipv4.addresses, ipv4.gateway)
                    .await?;
            }
            _ => {}
        }

        if let Some(ipv6) = ipv6_cfg {
            self.configure_ipv6_for_interface(iface_name, index, ipv6, cmd_tx)
                .await?;
        }

        kmsg::info!("Ethernet interface configured: {}", iface_name);
        Ok(())
    }

    async fn configure_ipv6_for_interface(
        &mut self,
        iface_name: &str,
        index: u32,
        ipv6: &Ipv6InterfaceConfig,
        cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) -> Result<()> {
        if !ipv6.addresses.is_empty() {
            self.apply_static_ipv6(iface_name, index, &ipv6.addresses, ipv6.gateway)
                .await?;
        } else if ipv6.autoconf && config::network().ipv6 {
            self.try_acquire_slaac(iface_name, cmd_tx).await;
        }
        Ok(())
    }
}
