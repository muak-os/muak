//! Bridge provisioning helpers for the network supervisor.

use anyhow::Result;
use config::system::network::{BridgeConfig, InterfaceKind};
use netdev::Ops;
use netlink::link::ifname::Name;
use tokio::sync::oneshot;

use super::super::NetworkSupervisor;
use super::{bridge_port_handle, send_command};
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;

// Creates every configured bridge before any address is applied to its owner.
pub(super) async fn configure_all<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.kind != InterfaceKind::Bridge {
            continue;
        }
        let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();
        if let Err(e) = configure(supervisor, &iface_cfg.name, &bridge_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

/// Points an existing bridge at its resolved port, reusing the bridge actor when present and spawning it when absent.
pub(in crate::supervisor) async fn reattach<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let snapshot = request(supervisor, bridge_name, bridge_cfg)
        .await?
        .await??;
    if !supervisor.interfaces.contains_key(&snapshot.name) {
        supervisor.spawn_interface_actor(snapshot);
    }

    let bridge = Name::new(bridge_name)?;
    supervisor.send_to_interface(&bridge, Command::LinkUp).await;

    Ok(())
}

async fn configure<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let snapshot = request(supervisor, bridge_name, bridge_cfg)
        .await?
        .await??;
    supervisor.spawn_interface_actor(snapshot);

    Ok(())
}

// Asks a bridge port actor to attach its bridge and returns the bridge snapshot reply.
async fn request<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<oneshot::Receiver<Result<Snapshot>>> {
    let (port_iface_name, _) = bridge_port_handle(supervisor, bridge_cfg)?;

    let actor_handle = supervisor
        .interfaces
        .get(&port_iface_name)
        .ok_or_else(|| anyhow::anyhow!("bridge port '{port_iface_name}' not found"))?;

    let (reply_tx, reply_rx) = oneshot::channel();
    send_command(
        actor_handle,
        Command::ConfigureBridge {
            bridge_name: bridge_name.to_owned(),
            stp: bridge_cfg.stp,
            reply: reply_tx,
        },
        &port_iface_name,
    )
    .await?;

    Ok(reply_rx)
}
