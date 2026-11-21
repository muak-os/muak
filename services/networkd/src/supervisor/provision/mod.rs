//! Applies declarative interface configuration from the config file via interface actors.

pub(super) mod addresses;
pub(super) mod bridge;

use alloc::borrow::Cow;
use alloc::sync::Arc;

use anyhow::Result;
use config::system::network::{BridgeConfig, InterfaceKind};
use netdev::Ops;
use netlink::link::ifname::Name;
use tokio::sync::watch;

use super::NetworkSupervisor;
use crate::interface::ActorHandle;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;

/// Applies declarative interface configuration from the config file.
pub(super) async fn interfaces<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    bridge::configure_all(supervisor).await;
    addresses::apply_all(supervisor).await;
}

/// Resolves a configured interface name, expanding the `auto` alias.
pub(super) fn resolve_name<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &str) -> Result<Name> {
    let primary = supervisor.get_primary_name()?;
    if name == "auto" {
        Ok(primary)
    } else {
        Name::new(name).map_err(Into::into)
    }
}

/// Returns the bridge that owns `name` as a port, or `name` itself.
pub(super) fn resolve_owner<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> Name {
    let Ok(primary) = supervisor.get_primary_name() else {
        return name.clone();
    };

    for iface_cfg in &supervisor.config.interfaces {
        if iface_cfg.kind != InterfaceKind::Bridge {
            continue;
        }
        let Some(bridge_cfg) = iface_cfg.bridge.as_ref() else {
            continue;
        };
        if resolve_bridge_port(&bridge_cfg.port, &primary).as_ref() == name.as_str()
            && let Ok(bridge_name) = Name::new(iface_cfg.name.as_str())
        {
            return bridge_name;
        }
    }

    name.clone()
}

/// Returns the kernel index of a runtime interface actor.
pub(super) fn actor_index<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &Name) -> Result<u32> {
    Ok(supervisor
        .interfaces
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("interface actor not found: {name}"))?
        .state_rx
        .borrow()
        .index)
}

pub(super) fn bridge_port_handle<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    bridge_cfg: &BridgeConfig,
) -> Result<(Name, watch::Receiver<Arc<Snapshot>>)> {
    let primary = supervisor.get_primary_name()?;
    let port_name = resolve_bridge_port(&bridge_cfg.port, &primary);
    let port_iface_name = Name::new(&*port_name)?;
    let actor_handle = supervisor
        .interfaces
        .get(&port_iface_name)
        .ok_or_else(|| anyhow::anyhow!("bridge port '{port_name}' not found"))?;

    Ok((port_iface_name, actor_handle.state_rx.clone()))
}

/// Sends a command to an interface actor, mapping a closed channel to an error.
pub(super) async fn send_command(
    handle: &ActorHandle,
    cmd: Command,
    iface_name: &Name,
) -> Result<()> {
    handle
        .cmd_tx
        .send(cmd)
        .await
        .map_err(|error| anyhow::anyhow!("interface actor gone: {iface_name}: {error}"))?;
    Ok(())
}

fn resolve_bridge_port<'a>(ports: &'a [String], primary: &'a Name) -> Cow<'a, str> {
    if ports.len() > 1 {
        kmsg::warn!(
            "bridge.port has {} entries; only the first is used \
             (multi-port bridges not yet supported)",
            ports.len()
        );
    }

    match ports.first() {
        Some(port) if port == "auto" => Cow::Borrowed(primary.as_str()),
        Some(port) => Cow::Borrowed(port.as_str()),
        None => Cow::Borrowed(primary.as_str()),
    }
}
