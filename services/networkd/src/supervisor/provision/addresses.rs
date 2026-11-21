//! Address provisioning helpers for the network supervisor.

use anyhow::Result;
use config::system::network::{InterfaceConfig, Ipv4InterfaceConfig, Ipv6InterfaceConfig};
use netdev::Ops;
use netlink::link::ifname::Name;

use super::super::NetworkSupervisor;
use super::{actor_index, resolve_name, resolve_owner, send_command};
use crate::interface::commands::{ApplyMode, Command};

// Applies every configured address to the interface that owns L3.
pub(super) async fn apply_all<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.ipv4.is_none() && iface_cfg.ipv6.is_none() {
            continue;
        }
        if let Err(e) = apply(supervisor, iface_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

async fn apply<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<()> {
    let name = resolve_name(supervisor, &iface_cfg.name)?;
    let owner = resolve_owner(supervisor, &name);
    supervisor.ops.ensure_up(name.as_str()).await?;

    if let Some(ipv4_cfg) = iface_cfg.ipv4.as_ref() {
        let index = actor_index(supervisor, &owner)?;
        ipv4(supervisor, &owner, index, ipv4_cfg).await?;
    }

    if let Some(ipv6_cfg) = iface_cfg.ipv6.as_ref() {
        let index = actor_index(supervisor, &name)?;
        ipv6(supervisor, &name, index, ipv6_cfg).await?;
    }

    Ok(())
}

async fn ipv4<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    iface_name: &Name,
    index: u32,
    ipv4_cfg: &Ipv4InterfaceConfig,
) -> Result<()> {
    let actor_handle = supervisor
        .interfaces
        .get(iface_name)
        .ok_or_else(|| anyhow::anyhow!("interface actor not found: {iface_name}"))?;

    kmsg::info!("Configuring IPv4 on interface: {iface_name}");

    if ipv4_cfg.dhcp {
        send_command(
            actor_handle,
            Command::ConfigureDhcp {
                mode: ApplyMode::Provision,
            },
            iface_name,
        )
        .await?;
        return Ok(());
    }

    if !ipv4_cfg.addresses.is_empty() {
        send_command(
            actor_handle,
            Command::ConfigureStaticIpv4 {
                mode: ApplyMode::Provision,
                index,
                addresses: ipv4_cfg.addresses.clone(),
                gateway: ipv4_cfg.gateway,
            },
            iface_name,
        )
        .await?;
    }

    Ok(())
}

async fn ipv6<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    iface_name: &Name,
    index: u32,
    ipv6_cfg: &Ipv6InterfaceConfig,
) -> Result<()> {
    let actor_handle = supervisor
        .interfaces
        .get(iface_name)
        .ok_or_else(|| anyhow::anyhow!("interface actor not found: {iface_name}"))?;

    if !ipv6_cfg.addresses.is_empty() {
        send_command(
            actor_handle,
            Command::ConfigureStaticIpv6 {
                mode: ApplyMode::Provision,
                index,
                addresses: ipv6_cfg.addresses.clone(),
                gateway: ipv6_cfg.gateway,
            },
            iface_name,
        )
        .await?;
        return Ok(());
    }
    if ipv6_cfg.autoconf && supervisor.config.ipv6 {
        send_command(
            actor_handle,
            Command::ConfigureSlaac {
                mode: ApplyMode::Provision,
            },
            iface_name,
        )
        .await?;
    }

    Ok(())
}
