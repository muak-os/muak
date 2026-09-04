//! Applies declarative interface configuration from the config file via interface actors.

use alloc::borrow::Cow;
use alloc::sync::Arc;

use anyhow::Result;
use config::{BridgeConfig, InterfaceKind, Ipv4InterfaceConfig, Ipv6InterfaceConfig};
use netlib::interface::Name;
use netlib::netlink::Ops;
use tokio::sync::oneshot;
use tokio::sync::watch;

use super::NetworkSupervisor;
use crate::interface::ActorHandle;
use crate::interface::commands::ApplyMode;
use crate::interface::commands::Command;
use crate::interface::snapshot::Snapshot;
use crate::interface::state::Lifecycle;

/// Applies declarative interface configuration from the config file.
pub(super) async fn interfaces<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        let result = match iface_cfg.kind {
            InterfaceKind::Bridge => {
                let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();
                bridge(supervisor, &iface_cfg.name, &bridge_cfg).await
            }
            InterfaceKind::Ethernet => {
                ethernet(
                    supervisor,
                    &iface_cfg.name,
                    iface_cfg.ipv4.as_ref(),
                    iface_cfg.ipv6.as_ref(),
                )
                .await
            }
        };
        if let Err(e) = result {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

pub(super) async fn bridge<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &str,
    bridge_cfg: &BridgeConfig,
) -> Result<()> {
    let (port_iface_name, mut state_rx) = bridge_port_handle(supervisor, bridge_cfg)?;
    wait_for_configured(&mut state_rx).await?;

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

    let bridge_snapshot = reply_rx.await??;
    supervisor.spawn_interface_actor(bridge_snapshot);

    Ok(())
}

/// Returns the configured bridge port actor and a watch receiver for its state.
pub(super) fn resolve_name<N: Ops>(supervisor: &NetworkSupervisor<N>, name: &str) -> Result<Name> {
    let primary = supervisor.get_primary_name()?;
    if name == "auto" {
        Ok(primary)
    } else {
        Name::new(name).map_err(Into::into)
    }
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

/// Resolves a configured interface name, expanding the `auto` alias.
async fn ethernet<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    name: &str,
    ipv4_cfg: Option<&Ipv4InterfaceConfig>,
    ipv6_cfg: Option<&Ipv6InterfaceConfig>,
) -> Result<()> {
    let iface_name = resolve_name(supervisor, name)?;

    let actor_handle = supervisor
        .interfaces
        .get(&iface_name)
        .ok_or_else(|| anyhow::anyhow!("ethernet interface '{iface_name}' not found"))?;

    kmsg::info!("Configuring ethernet interface: {}", iface_name);
    let index = supervisor.ops.ensure_up(iface_name.as_str()).await?;

    match ipv4_cfg {
        Some(ipv4_cfg) if ipv4_cfg.dhcp => {
            send_command(
                actor_handle,
                Command::ConfigureDhcp {
                    mode: ApplyMode::Provision,
                },
                &iface_name,
            )
            .await?;
        }
        Some(ipv4_cfg) if !ipv4_cfg.addresses.is_empty() => {
            send_command(
                actor_handle,
                Command::ConfigureStaticIpv4 {
                    mode: ApplyMode::Provision,
                    index,
                    addresses: ipv4_cfg.addresses.clone(),
                    gateway: ipv4_cfg.gateway,
                },
                &iface_name,
            )
            .await?;
        }
        _ => {}
    }

    if let Some(cfg) = ipv6_cfg {
        ipv6(supervisor, &iface_name, index, cfg).await?;
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

/// Waits until the interface actor reports `Configured`.
async fn wait_for_configured(rx: &mut watch::Receiver<Arc<Snapshot>>) -> Result<()> {
    loop {
        if rx.borrow().state == Lifecycle::Configured {
            return Ok(());
        }
        rx.changed().await.map_err(|error| {
            anyhow::anyhow!("interface actor dropped before reaching Configured: {error}")
        })?;
    }
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
