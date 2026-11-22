//! Applies declarative interface configuration from the config file.

pub(super) mod addresses;
pub(super) mod bridge;

use alloc::borrow::Cow;

use anyhow::Result;
use config::system::network::{BridgeConfig, InterfaceKind};
use linkctl::interface::{Command, Handle};
use netdev::Ops;
use netlink::link::ifname::Name;

use super::NetworkSupervisor;

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

/// Returns the runtime handle for `name`, or an error when it is absent.
pub(super) fn handle<'a, N: Ops>(
    supervisor: &'a NetworkSupervisor<N>,
    name: &Name,
) -> Result<&'a Handle> {
    supervisor
        .interfaces
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("interface not found: {name}"))
}

/// Resolves the port a bridge should attach to, expanding the `auto` alias.
pub(super) fn bridge_port<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    bridge_cfg: &BridgeConfig,
) -> Result<Name> {
    let primary = supervisor.get_primary_name()?;
    let port_name = resolve_bridge_port(&bridge_cfg.port, &primary);

    Name::new(&*port_name).map_err(Into::into)
}

/// Sends a command to an interface handle, mapping a closed channel to an error.
pub(super) async fn send_command(handle: &Handle, cmd: Command, name: &Name) -> Result<()> {
    handle
        .command_tx
        .send(cmd)
        .await
        .map_err(|error| anyhow::anyhow!("interface handle gone: {name}: {error}"))?;

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
