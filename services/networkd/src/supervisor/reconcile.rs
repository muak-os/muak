//! Periodic reconciliation of configured network intent.

use anyhow::Result;
use config::system::network;
use netdev::Ops;
use netlink::link::ifname::Name;
use netstack::interface::Command;
use network::{InterfaceConfig, InterfaceKind};

use super::{NetworkSupervisor, dns, provision, view};

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

    reapply(supervisor, &target, iface_cfg).await?;

    Ok(ReconcileDisposition::Applied)
}

/// Reapplies a bridge intent by re-attaching its port and its own addresses.
async fn bridge<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<ReconcileDisposition> {
    let bridge_name = Name::new(iface_cfg.name.as_str())?;
    let bridge_cfg = iface_cfg.bridge.clone().unwrap_or_default();

    println!("Reconciling bridge interface: {bridge_name}");
    provision::bridge::ensure(supervisor, bridge_name.as_str(), &bridge_cfg).await?;
    reapply(supervisor, &bridge_name, iface_cfg).await?;

    Ok(ReconcileDisposition::Applied)
}

// Reconciles a configured interface, or provisions a handle that lost its methods.
async fn reapply<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    name: &Name,
    iface_cfg: &InterfaceConfig,
) -> Result<()> {
    if iface_cfg.ipv4.is_none() && iface_cfg.ipv6.is_none() {
        return Ok(());
    }

    let handle = provision::handle(supervisor, name)?;
    let active = view::is_active(&handle.snapshot_rx.borrow());
    if active {
        return provision::send_command(handle, Command::Reconcile, name).await;
    }

    provision::addresses::configure(supervisor, name, iface_cfg).await
}
