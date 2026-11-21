//! Periodic reconciliation of configured network intent.

use anyhow::Result;
use config::system::network;
use netdev::Ops;
use netlink::link::ifname::Name;
use netlink::link::state::State;
use network::{BridgeConfig, InterfaceConfig, InterfaceKind};

use super::{NetworkSupervisor, dns, provision};
use crate::interface::commands::{ApplyMode, Command};
use crate::interface::lifecycle::Lifecycle;
use crate::interface::snapshot::Snapshot;

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

/// Reapplies a bridge intent and refreshes the bridge actor when needed.
async fn bridge<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<ReconcileDisposition> {
    let bridge_name = Name::new(iface_cfg.name.as_str())?;
    let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();

    let is_configured = supervisor
        .interfaces
        .get(&bridge_name)
        .is_some_and(|handle| handle.state_rx.borrow().state == Lifecycle::Configured);

    if is_configured {
        return Ok(ReconcileDisposition::Skipped(
            "bridge is already configured".to_owned(),
        ));
    }

    if supervisor.interfaces.contains_key(&bridge_name) {
        kmsg::info!("Reconciling bridge interface: {}", bridge_name);
        let bridge_snapshot = bridge_snapshot(supervisor, &bridge_name, &bridge_cfg).await?;
        provision::respawn_actor(supervisor, bridge_snapshot).await;
        own_addresses(supervisor, iface_cfg, &bridge_name).await?;

        return Ok(ReconcileDisposition::Applied);
    }

    let Some(port_iface_name) = ready_bridge_port(supervisor, &bridge_cfg) else {
        return Ok(ReconcileDisposition::Skipped(
            "bridge port is not ready".to_owned(),
        ));
    };

    println!("Reconciling bridge interface: {bridge_name} via port {port_iface_name}");
    provision::bridge(supervisor, bridge_name.as_str(), &bridge_cfg).await?;
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

/// Reapplies bridge configuration and returns the refreshed bridge snapshot.
async fn bridge_snapshot<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    bridge_name: &Name,
    bridge_cfg: &BridgeConfig,
) -> Result<Snapshot> {
    let bridge_handle = supervisor
        .interfaces
        .get(bridge_name)
        .ok_or_else(|| anyhow::anyhow!("bridge interface '{bridge_name}' not found"))?;
    let bridge_snapshot = bridge_handle.state_rx.borrow().clone();
    let (port_iface_name, _) = provision::bridge_port_handle(supervisor, bridge_cfg)?;
    let mac = supervisor
        .ops
        .ensure_bridge(
            bridge_name.as_str(),
            port_iface_name.as_str(),
            bridge_cfg.stp,
        )
        .await?;

    let index = supervisor.ops.index(bridge_name.as_str()).await?;

    Ok(Snapshot {
        name: bridge_name.clone(),
        state: Lifecycle::Configured,
        index,
        mac,
        link: State::Up,
        ip: bridge_snapshot.ip.clone(),
        lease: bridge_snapshot.lease.clone(),
        dhcp_state: bridge_snapshot.dhcp_state.clone(),
        ipv6: bridge_snapshot.ipv6.clone(),
        l3_owner: bridge_name.clone(),
    })
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

/// Returns the bridge port name when the port is configured enough to reconcile the bridge.
fn ready_bridge_port<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    bridge_cfg: &BridgeConfig,
) -> Option<Name> {
    let Ok((port_iface_name, state_rx)) = provision::bridge_port_handle(supervisor, bridge_cfg)
    else {
        return None;
    };
    let snap = state_rx.borrow();
    if snap.link == State::Up {
        return Some(port_iface_name);
    }

    None
}
