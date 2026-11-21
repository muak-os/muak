//! Periodic reconciliation of configured network intent.

use anyhow::Result;
use config::system::network;
use netdev::Ops;
use netlink::link::ifname::Name;
use network::{InterfaceConfig, InterfaceKind};

use super::{NetworkSupervisor, dns, provision};
use crate::interface::commands::{ApplyMode, Command};

/// Describes whether reconcile applied intent or skipped it deliberately.
enum ReconcileDisposition {
    Applied,
    Skipped(String),
}

/// Reapplies declarative interface configuration to converge drifted state.
pub(super) async fn run<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    if !supervisor.state.is_operational() {
        return;
    }

    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        let disposition = match iface_cfg.kind {
            InterfaceKind::Bridge => bridge(supervisor, iface_cfg).await,
            InterfaceKind::Ethernet => ethernet(supervisor, iface_cfg).await,
        };
        match disposition {
            Ok(ReconcileDisposition::Applied) => {}
            Ok(ReconcileDisposition::Skipped(reason)) => {
                println!("Skipping reconcile for {}: {}", iface_cfg.name, reason);
            }
            Err(e) => {
                kmsg::warn!("Failed to reconcile {}: {}", iface_cfg.name, e);
            }
        }
    }

    dns::flush(supervisor);
}

/// Reapplies an Ethernet interface intent on the owner that holds L3.
async fn ethernet<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<ReconcileDisposition> {
    let iface_name = provision::resolve_name(supervisor, &iface_cfg.name)?;
    if !supervisor.interfaces.contains_key(&iface_name) {
        return Ok(ReconcileDisposition::Skipped(format!(
            "interface '{iface_name}' is not known at runtime"
        )));
    }

    let target = provision::resolve_owner(supervisor, &iface_name);

    println!("Reconciling ethernet interface: {iface_name}");
    supervisor.ops.ensure_up(iface_name.as_str()).await?;

    if let Some(ipv4_cfg) = iface_cfg.ipv4.as_ref() {
        let index = provision::actor_index(supervisor, &target)?;
        ipv4(supervisor, &target, index, ipv4_cfg).await?;
    }

    if let Some(ipv6_cfg) = iface_cfg.ipv6.as_ref() {
        let index = provision::actor_index(supervisor, &iface_name)?;
        ipv6(supervisor, &iface_name, index, ipv6_cfg).await?;
    }

    Ok(ReconcileDisposition::Applied)
}

/// Reapplies a bridge intent by re-pointing its actor at the resolved port.
async fn bridge<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<ReconcileDisposition> {
    let bridge_name = Name::new(iface_cfg.name.as_str())?;
    let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();

    println!("Reconciling bridge interface: {bridge_name}");
    provision::bridge::reattach(supervisor, bridge_name.as_str(), &bridge_cfg).await?;
    own_addresses(supervisor, iface_cfg, &bridge_name).await?;

    Ok(ReconcileDisposition::Applied)
}

/// Reapplies the address intent declared on the bridge entry to the bridge itself.
async fn own_addresses<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
    bridge_name: &Name,
) -> Result<()> {
    if iface_cfg.ipv4.is_none() && iface_cfg.ipv6.is_none() {
        return Ok(());
    }

    let index = provision::actor_index(supervisor, bridge_name)?;

    if let Some(ipv4_cfg) = iface_cfg.ipv4.as_ref() {
        ipv4(supervisor, bridge_name, index, ipv4_cfg).await?;
    }

    if let Some(ipv6_cfg) = iface_cfg.ipv6.as_ref() {
        ipv6(supervisor, bridge_name, index, ipv6_cfg).await?;
    }

    Ok(())
}

/// Reapplies IPv4 configuration for one interface.
async fn ipv4<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    iface_name: &Name,
    index: u32,
    ipv4_cfg: &network::Ipv4InterfaceConfig,
) -> Result<()> {
    let actor_handle = supervisor
        .interfaces
        .get(iface_name)
        .ok_or_else(|| anyhow::anyhow!("interface actor not found: {iface_name}"))?;

    if ipv4_cfg.dhcp {
        println!("Reconciling DHCP on {iface_name}");
        provision::send_command(
            actor_handle,
            Command::ConfigureDhcp {
                mode: ApplyMode::Reconcile,
            },
            iface_name,
        )
        .await?;
        return Ok(());
    }

    if !ipv4_cfg.addresses.is_empty() {
        println!("Reconciling static IPv4 on {iface_name}");
        provision::send_command(
            actor_handle,
            Command::ConfigureStaticIpv4 {
                mode: ApplyMode::Reconcile,
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

/// Reapplies IPv6 configuration for one interface.
async fn ipv6<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    iface_name: &Name,
    index: u32,
    ipv6_cfg: &network::Ipv6InterfaceConfig,
) -> Result<()> {
    let actor_handle = supervisor
        .interfaces
        .get(iface_name)
        .ok_or_else(|| anyhow::anyhow!("interface actor not found: {iface_name}"))?;

    if !ipv6_cfg.addresses.is_empty() {
        println!("Reconciling static IPv6 on {iface_name}");
        provision::send_command(
            actor_handle,
            Command::ConfigureStaticIpv6 {
                mode: ApplyMode::Reconcile,
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
        println!("Reconciling SLAAC on {iface_name}");
        provision::send_command(
            actor_handle,
            Command::ConfigureSlaac {
                mode: ApplyMode::Reconcile,
            },
            iface_name,
        )
        .await?;
    }

    Ok(())
}
