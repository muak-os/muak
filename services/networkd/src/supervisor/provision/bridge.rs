//! Bridge provisioning helpers for the network supervisor.

use anyhow::Result;
use config::system::network::{BridgeConfig, InterfaceKind};
use netdev::Ops;
use netdev::interface::Ethernet;
use netlink::link::ifname::Name;

use super::super::{NetworkSupervisor, carrier};
use super::bridge_port;

// Creates every configured bridge before any address is applied to its owner.
pub(super) async fn configure_all<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.kind != InterfaceKind::Bridge {
            continue;
        }
        let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();
        if let Err(e) = ensure(supervisor, &iface_cfg.name, &bridge_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

/// Ensures the kernel bridge exists with its port attached, reusing its handle.
pub(in crate::supervisor) async fn ensure<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let port = bridge_port(supervisor, bridge_cfg)?;
    if !supervisor.interfaces.contains_key(&port) {
        anyhow::bail!("bridge port '{port}' not found");
    }
    let bridge = supervisor
        .ops
        .ensure_bridge(bridge_name, port.as_str(), bridge_cfg.stp)
        .await?;
    let index = supervisor.ops.index(bridge_name).await?;
    let bridge_name = Name::new(bridge_name)?;

    if !supervisor.interfaces.contains_key(&bridge_name) {
        let link = Ethernet::new(bridge_name.clone(), index, bridge.mac, bridge.link);
        supervisor.spawn_interface(link);
        kmsg::info!(
            "Bridge {bridge_name} ready with port {} (index {index})",
            port.as_str()
        );
    }

    supervisor.owners.insert(port, bridge_name.clone());
    supervisor
        .owners
        .insert(bridge_name.clone(), bridge_name.clone());
    supervisor.state.l3_owner = Some(bridge_name);

    carrier::refresh_owner(supervisor).await;

    Ok(())
}
