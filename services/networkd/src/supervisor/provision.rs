//! Applies declarative interface configuration from the config file via interface actors.

use alloc::borrow::Cow;
use alloc::sync::Arc;

use anyhow::Result;
use config::system::network::{
    BridgeConfig, InterfaceConfig, InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig,
};
use netdev::Ops;
use netlink::link::ifname::Name;
use tokio::sync::{oneshot, watch};

use super::NetworkSupervisor;
use crate::interface::ActorHandle;
use crate::interface::commands::{ApplyMode, Command};
use crate::interface::snapshot::Snapshot;

/// Applies declarative interface configuration from the config file.
pub(super) async fn interfaces<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    bridges(supervisor).await;
    addresses(supervisor).await;
}

pub(super) async fn bridge<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let bridge_snapshot = configure_bridge(supervisor, bridge_name, bridge_cfg)
        .await?
        .await??;
    supervisor.spawn_interface_actor(bridge_snapshot);

    Ok(())
}

/// Points an existing bridge at its resolved port, reusing the bridge actor when present and spawning it when absent.
pub(super) async fn reattach<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let bridge_snapshot = configure_bridge(supervisor, bridge_name, bridge_cfg)
        .await?
        .await??;
    if !supervisor.interfaces.contains_key(&bridge_snapshot.name) {
        supervisor.spawn_interface_actor(bridge_snapshot);
    }

    // The bridge's carrier returns when the new port is attached, but the monitor
    // only reports ethernet names, so the bridge never gets its own link-up event.
    let bridge = Name::new(bridge_name)?;
    supervisor.send_to_interface(&bridge, Command::LinkUp).await;

    Ok(())
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

/// Replaces an existing interface actor with a fresh snapshot.
pub(super) async fn respawn_actor<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    snapshot: Snapshot,
) {
    if let Some(existing) = supervisor.interfaces.remove(&snapshot.name) {
        drop(existing.cmd_tx.send(Command::Shutdown).await);
    }
    supervisor.spawn_interface_actor(snapshot);
}

// Asks a bridge port actor to attach its bridge and returns the bridge snapshot reply.
async fn configure_bridge<N: Ops>(
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

// Creates every configured bridge before any address is applied to its owner.
async fn bridges<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.kind != InterfaceKind::Bridge {
            continue;
        }
        let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();
        if let Err(e) = bridge(supervisor, &iface_cfg.name, &bridge_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

// Applies every configured address to the interface that owns L3.
async fn addresses<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if iface_cfg.ipv4.is_none() && iface_cfg.ipv6.is_none() {
            continue;
        }
        if let Err(e) = apply_addresses(supervisor, iface_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

async fn apply_addresses<N: Ops>(
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
        // IPv6 follows the port until 5B moves it to the bridge.
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
